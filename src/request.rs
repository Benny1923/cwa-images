use bytes::Bytes;
use tokio::io::AsyncWrite;
use url::Url;

pub async fn download(client: &mut reqwest::Client, url: Url) -> reqwest::Result<Bytes> {
    let resp = client.get(url).send().await?.error_for_status()?;

    resp.bytes().await
}
