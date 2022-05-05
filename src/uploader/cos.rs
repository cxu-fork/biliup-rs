use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;
use reqwest::{Body, header};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::RetryTransientMiddleware;
use anyhow::{anyhow, bail, Result};
use bytes::Bytes;
use futures::{Stream, StreamExt, TryStreamExt};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::Video;

pub struct Cos {
    client: ClientWithMiddleware,
    raw_client: reqwest::Client,
    bucket: Bucket,
    // url: String,
    upload_id: String,
}

impl Cos {
    pub async fn form_post(bucket: Bucket) -> Result<Cos> {
        // let mut headers = header::HeaderMap::new();
        // headers.insert("Authorization", header::HeaderValue::from_str(&bucket.post_auth)?);
        let raw_client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/63.0.3239.108")
            // .default_headers(headers)
            .timeout(Duration::new(300, 0))
            .build()
            .unwrap();
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(5);
        let client = ClientBuilder::new(raw_client.clone())
            // Retry failed requests.
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();
        // let url = format!(
        //     "https:{}/{}",
        //     bucket.endpoint,
        //     bucket.upos_uri.replace("upos://", "")
        // ); // 视频上传路径
        // let upload_id: serde_json::Value = client
        //     .post(format!("{url}?uploads&output=json"))
        //     .send()
        //     .await?
        //     .json()
        //     .await?;
        // client.post("aaa")
        let upload_id = get_uploadid(&client, &bucket).await?;
        Ok(Cos {
            client,
            raw_client,
            bucket,
            upload_id
        })
    }



    pub async fn upload_stream<F, B>(&self, stream: F,total_size: u64,limit: usize, enable_internal: bool) -> Result<Vec<(usize, String)>>
        where
            F: Stream<Item = Result<(B, usize)>>,
            B: Into<Body> + Clone
            // Body: From<B>
    {
        let chunk_size = 10485760;
        let chunks_num = (total_size as f64 / chunk_size as f64).ceil() as u32; // 获取分块数量
        // let file = tokio::io::BufReader::with_capacity(chunk_size, file);
        let client = &self.raw_client;
        let mut temp;
        let url = if enable_internal {
            temp = self.bucket.url.replace("cos.accelerate", "cos-internal.ap-shanghai");
            &temp
        } else {
            &self.bucket.url
        };
        let upload_id = &self.upload_id;
        let stream = stream
            // let mut chunks = read_chunk(file, chunk_size)
            .enumerate()
            .map(move |(i, chunk)| async move {
                let (chunk, len) = chunk?;
                // let len = chunk.len();
                // println!("{}", len);
                let params = Protocol {
                    upload_id,
                    part_number: (i + 1) as u32,
                };
                let response = super::retryable::retry(|| async {
                    let response = client.put(url)
                        .header(AUTHORIZATION, &self.bucket.put_auth)
                        .header(CONTENT_LENGTH, len)
                        .query(&params).body(chunk.clone()).send().await?;
                    response.error_for_status_ref();
                    Ok::<_, reqwest::Error>(response)
                }).await?;

                // json!({"partNumber": i + 1, "eTag": response.headers().get("Etag")})
                let headers = response.headers();
                let etag = match headers.get("Etag") {
                    None => bail!("upload chunk {i} error: {}", response.text().await?),
                    Some(etag) => etag.to_str()?.to_string()
                };
                // etag.ok_or(anyhow!("{res}")).map(|s|s.to_str())??.to_string()
                // let res = response.text().await?;
                Ok::<_, anyhow::Error>((i + 1, etag))
            })
            .buffer_unordered(limit);
        let mut parts = Vec::new();
        tokio::pin!(stream);
        while let Some((part, etag)) = stream.try_next().await? {
            parts.push((part, etag));
            // if !process(size) {
            //     bail!("移除视频");
            // }
        }
        Ok(parts)
    }

    pub async fn merge_files(&self, mut parts: Vec<(usize, String)>) -> Result<Video> {
        parts.sort_unstable_by_key(|annotate| annotate.0);
        // let complete_multipart_upload
        let complete_multipart_upload = parts.iter().map(|(number, etag)| format!(r#"
    <Part>
        <PartNumber>{number}</PartNumber>
        <ETag>{etag}</ETag>
    </Part>
    "#)).reduce(|accum, item| accum + &item).unwrap();
        let xml = format!(r#"
    <CompleteMultipartUpload>
    {complete_multipart_upload}
    </CompleteMultipartUpload>
    "#);
        let mut headers = header::HeaderMap::new();
        headers.insert("Authorization", header::HeaderValue::from_str(&self.bucket.post_auth)?);
        let response = self.client.post(&self.bucket.url).query(&[("uploadId", &self.upload_id)])
            .body(xml).headers(headers).send().await?;
        if !response.status().is_success() {
            bail!(response.text().await?)
        }
        let mut headers = header::HeaderMap::new();
        headers.insert("X-Upos-Fetch-Source", header::HeaderValue::from_str(&self.bucket.fetch_headers.get("X-Upos-Fetch-Source").unwrap())?);
        headers.insert("X-Upos-Auth", header::HeaderValue::from_str(&self.bucket.fetch_headers.get("X-Upos-Auth").unwrap())?);
        headers.insert("Fetch-Header-Authorization", header::HeaderValue::from_str(&self.bucket.fetch_headers.get("Fetch-Header-Authorization").unwrap())?);
        let res = self.client.post(format!("https:{}", self.bucket.fetch_url)).headers(headers).send().await?;
        if !res.status().is_success() {
            bail!( res.text().await?)
        }
        Ok(Video{
            title: None,
            filename: Path::new(&self.bucket.bili_filename)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
            desc: "".into()
        })
    }
}


async fn get_uploadid(client: &ClientWithMiddleware, bucket: &Bucket) -> Result<String> {
    let res = client.post(format!("{}?uploads&output=json", bucket.url))
        .header(reqwest::header::AUTHORIZATION, &bucket.post_auth)
        .send().await?
        .text().await?;
    let start = res.find(r"<UploadId>").ok_or(anyhow!("{res}"))? + "<UploadId>".len();
    let end = res.rfind(r"</UploadId>").ok_or(anyhow!("{res}"))?;
    let uploadid = &res[start..end];
    Ok(uploadid.to_string())
}



#[derive(Serialize, Deserialize, Debug)]
pub struct Bucket {
    #[serde(rename = "OK")]
    ok: u8,
    bili_filename: String,
    biz_id: usize,
    fetch_headers: HashMap<String, String>,
    fetch_url: String,
    fetch_urls: Vec<String>,
    post_auth: String,
    put_auth: String,
    url: String,
}

// #[derive(Serialize, Deserialize, Debug)]
// pub struct Bucket {
//     chunk_size: usize,
//     auth: String,
//     endpoint: String,
//     biz_id: usize,
//     upos_uri: String,
// }

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Protocol<'a> {
    upload_id: &'a str,
    part_number: u32,
}