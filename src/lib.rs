use crate::video::{Studio, Video};
use anyhow::Result;
use async_std::fs::File;
use async_stream::try_stream;
use bytes::{BufMut, Bytes, BytesMut};
use futures::{AsyncReadExt, Stream};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub mod client;
pub mod error;
pub mod line;
pub mod video;

pub mod uploader {
    use serde::{Deserialize, Serialize};
    pub mod kodo;
    pub mod upos;

    #[derive(Deserialize, Serialize, Debug)]
    #[serde(rename_all = "lowercase")]
    pub enum Uploader {
        Upos,
        Kodo,
        Bos,
        Gcs,
        Cos,
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub account: Account,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub user: Option<User>,
    pub line: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub streamers: HashMap<String, Studio>,
}

fn default_limit() -> usize {
    3
}

pub fn load_config(config: &Path) -> Result<Config> {
    let file = std::fs::File::open(config)?;
    let config: Config = serde_yaml::from_reader(file)?;
    // println!("body = {:?}", client);
    Ok(config)
}

pub(crate) fn read_chunk(mut file: File, len: usize) -> impl Stream<Item = Result<Bytes>> {
    let mut buffer = vec![0u8; len];

    let mut buf = BytesMut::with_capacity(len);
    try_stream! {
        loop {
            let n = file.read(&mut buffer).await?;
            buf.put_slice(&buffer[..n]);
        // println!("{:?}", buf);
            if n == 0 {
                return;
            }
            yield buf.split().freeze();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::client::Client;
    use crate::video::{BiliBili, Studio, Video};
    use anyhow::Result;

    #[tokio::test]
    async fn it_works() -> Result<()> {
        println!("yes");
        Ok(())
    }
}
