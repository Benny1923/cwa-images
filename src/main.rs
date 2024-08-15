use clap::Parser;
use lazy_static::lazy_static;
use log::{debug, error, info, warn, LevelFilter};
use parser::{find_objects, parse_source, CondKeys};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use std::io::{self, Cursor};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::fs::{remove_file, File};
use tokio::io::{copy, AsyncRead};
use tokio::time;
use url::Url;

mod parser;

lazy_static! {
    static ref CWA_HOST: String = env::var("CWA_HOST").unwrap_or(DEFAULT_CWA_HOST.to_string());
}

const DEFAULT_CWA_HOST: &str = "https://www.cwa.gov.tw";

const OBSERVE_SAT_LIST: &str = "/Data/js/obs_img/Observe_sat.js";
const OBSERVE_SAT_DIR: &str = "/Data/satellite/";

const OBSERVE_RADAR_LIST: &str = "/Data/js/obs_img/Observe_radar.js";
const OBSERVE_RADAR_DIR: &str = "/Data/radar/";

const OBSERVE_RADAR_RAIN_LIST: &str = "/Data/js/obs_img/Observe_radar_rain.js";
const OBSERVE_RADAR_RAIN_DIR: &str = "/Data/radar_rain/";

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, help = "download file with contain string")]
    sat_img: Option<String>,
    #[arg(long, help = "download file with contain string")]
    radar_cloud: Option<String>,
    #[arg(long, help = "download file with contain string. e.g. RCLY_3600")]
    radar_rain: Option<String>,

    #[arg(
        long,
        help = "download file with contain string",
        help_heading = "Custom",
        requires("custom_list"),
        requires("custom_dir")
    )]
    custom: Option<String>,
    #[arg(
        long,
        help_heading = "Custom",
        help = "path of images list url. e.g. /Data/js/obs_img/Observe_lightning.js"
    )]
    custom_list: Option<String>,
    #[arg(
        long,
        help_heading = "Custom",
        help = "path of images dir. e.g. /Data/lightning/"
    )]
    custom_dir: Option<String>,

    #[arg(default_value = "images", help = "download dir")]
    dir: String,

    #[arg(
        long,
        short,
        default_value = "0",
        help = "job interval, unit: second, 0 is disable"
    )]
    interval: u64,

    #[arg(long, short, help = "print debug message")]
    debug: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Img {
    img: String,
    text: String,
}

impl Img {
    async fn download(
        &self,
        client: &mut Client,
        dir: &str,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let url = Url::from_str(&CWA_HOST)?.join(dir)?.join(&self.img)?;

        // tf?
        Ok(client.get(url).send().await?.error_for_status()?)
    }

    fn filename(&self) -> &str {
        Path::new(&self.img).file_name().unwrap().to_str().unwrap()
    }
}

impl CondKeys for Img {
    fn keys<'a>() -> &'a [&'a str] {
        &["img", "text"]
    }
}

#[derive(Debug)]
struct Task {
    list: String,
    dir: String,
    contains: String,
}

impl Task {
    fn new(list: String, dir: String, contains: String) -> Self {
        Self {
            list,
            dir,
            contains,
        }
    }

    fn new_sat(contains: String) -> Self {
        Self::new(
            OBSERVE_SAT_LIST.to_string(),
            OBSERVE_SAT_DIR.to_string(),
            contains,
        )
    }

    fn new_radar(contains: String) -> Self {
        Self::new(
            OBSERVE_RADAR_LIST.to_string(),
            OBSERVE_RADAR_DIR.to_string(),
            contains,
        )
    }

    fn new_radar_rain(contains: String) -> Self {
        Self::new(
            OBSERVE_RADAR_RAIN_LIST.to_string(),
            OBSERVE_RADAR_RAIN_DIR.to_string(),
            contains,
        )
    }

    async fn download_list(&self, client: &mut Client) -> Result<Vec<Img>, Box<dyn Error>> {
        info!("download list");
        let url = Url::from_str(&CWA_HOST)?.join(&self.list)?;
        debug!("list url {}", url);
        let source = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let object = parse_source(&source)?;
        Ok(find_objects(object))
    }

    async fn run(&self, client: &mut Client, out_dir: &Path) -> Result<(), Box<dyn Error>> {
        let list = self.download_list(client).await?;
        let target_imgs_iter = list.iter().filter(|x| x.img.contains(&self.contains));

        for img in target_imgs_iter {
            let dest = out_dir.join(img.filename());
            // skip exists file
            if dest.is_file() {
                debug!("skiped {}", dest.to_str().unwrap());
                continue;
            }
            let response = img.download(client, &self.dir).await;
            match response {
                Ok(resp) => {
                    if let Ok(bytes) = resp.bytes().await {
                        let mut reader = Cursor::new(bytes);
                        let _ = save_file(&dest, &mut reader).await;
                    }
                }
                Err(err) => {
                    error!("download image failed: {}", err);
                }
            };
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let mut logger = env_logger::builder();

    if args.debug {
        logger.filter_level(LevelFilter::Debug);
    } else {
        logger.filter_level(LevelFilter::Info);
    }

    logger.init();

    // setup dir
    debug!("setup dir...");
    let images_dir = Path::new(&args.dir);
    check_dir(images_dir).expect("can not create dir");

    // create task
    let mut tasks = Vec::new();

    if let Some(sat) = args.sat_img {
        tasks.push(Task::new_sat(sat));
    }

    if let Some(radar) = args.radar_cloud {
        tasks.push(Task::new_radar(radar));
    }

    if let Some(radar_rain) = args.radar_rain {
        tasks.push(Task::new_radar_rain(radar_rain));
    }

    if let Some(custom) = args.custom {
        tasks.push(Task::new(
            args.custom_list.expect("list args required"),
            args.custom_dir.expect("dir args required"),
            custom,
        ))
    }

    let cycle_time = if args.interval != 0 {
        Duration::from_secs(args.interval)
    } else {
        // dummy interval
        Duration::from_secs(3600)
    };
    let mut interval = time::interval(cycle_time);

    let mut client = Client::new();

    loop {
        interval.tick().await;

        info!("run tasks");
        for task in &tasks {
            match task.run(&mut client, images_dir).await {
                Ok(_) => {}
                Err(err) => {
                    error!("{}", err)
                }
            }
        }
        info!("tasks finished");

        if args.interval == 0 {
            break;
        }
    }

    info!("program exited");
}

#[inline]
fn check_dir(path: &Path) -> Result<(), std::io::Error> {
    if path.is_dir() {
        Ok(())
    } else {
        std::fs::create_dir_all(path)
    }
}

#[inline]
async fn save_file<R: AsyncRead + Unpin + ?Sized>(dest: &Path, reader: &mut R) -> io::Result<u64> {
    let mut file = File::create(dest).await?;

    let result = copy(reader, &mut file).await;

    match result {
        Ok(size) => {
            info!(
                "saved {} {}",
                dest.to_str().unwrap(),
                human_size(size as usize)
            );
        }
        Err(ref err) => {
            warn!("cannot save file {}", err);
            let _ = remove_file(dest).await;
        }
    }

    result
}

#[inline]
fn human_size(size: usize) -> String {
    let units = ['K', 'M', 'G', 'T'];
    let mut unit = ' ';
    let mut fsize = size as f64;
    for u in units {
        if fsize / 1024.0 < 1.0 {
            break;
        }

        fsize /= 1024.0;
        unit = u;
    }

    format!("{:.2}{}B", fsize, unit)
}
