use crate::downloader;
use crate::downloader::httpflv::Connection;
use crate::downloader::util::{LifecycleFile, Segmentable};
use crate::downloader::{hls, httpflv};
use async_trait::async_trait;
use reqwest::header::{HeaderValue, ACCEPT_ENCODING};
use std::any::Any;
use std::fmt::{Display, Formatter};

use crate::client::StatelessClient;

mod bilibili;
mod douyu;
mod huya;

const EXTRACTORS: [&(dyn SiteDefinition + Send + Sync); 3] = [
    &bilibili::BiliLive {},
    &huya::HuyaLive {},
    &douyu::DouyuLive,
];

#[async_trait]
pub trait SiteDefinition {
    // true, if this site can handle <url>.
    fn can_handle_url(&self, url: &str) -> bool;

    async fn get_site(&self, url: &str, client: StatelessClient) -> super::error::Result<Site>;

    fn as_any(&self) -> &dyn Any;
}

pub struct Site {
    pub name: &'static str,
    pub title: String,
    pub direct_url: String,
    extension: Extension,
    client: StatelessClient,
}

impl Display for Site {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Name: {}", self.name)?;
        writeln!(f, "Title: {}", self.title)?;
        write!(f, "Direct url: {}", self.direct_url)
    }
}

enum Extension {
    Flv,
    Ts,
}

impl Site {
    pub async fn download(
        &mut self,
        mut file: LifecycleFile,
        segment: Segmentable,
    ) -> downloader::error::Result<()> {
        file.fmt_file_name = file.fmt_file_name.replace("{title}", &self.title);
        self.client
            .headers
            .append(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));
        println!("{}", self);
        match self.extension {
            Extension::Flv => {
                let response = self.client.retryable(&self.direct_url).await?;
                let mut connection = Connection::new(response);
                connection.read_frame(9).await?;
                file.extension = "flv";
                httpflv::parse_flv(connection, file, segment).await?
            }
            Extension::Ts => {
                hls::download(&self.direct_url, &self.client, &file.fmt_file_name, segment).await?
            }
        }
        Ok(())
    }
}

pub fn find_extractor(url: &str) -> Option<&'static (dyn SiteDefinition + Send + Sync)> {
    EXTRACTORS
        .into_iter()
        .find(|&extractor| extractor.can_handle_url(url))
}
