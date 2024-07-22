use clap::Parser;
use log::{debug, error, info, warn, LevelFilter};
use parser::{find_objects, parse_source, CondKeys};
use reqwest::Client;
use serde::{Deserialize, Serialize};
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

const CWA_HOST: &str = "https://www.cwa.gov.tw";

const OBSERVE_SAT_LIST: &str = "/Data/js/obs_img/Observe_sat.js";
const OBSERVE_SAT_DIR: &str = "/Data/satellite/";

const OBSERVE_RADAR_LIST: &str = "/Data/js/obs_img/Observe_radar.js";
const OBSERVE_RADAR_DIR: &str = "/Data/radar/";

const OBSERVE_RADAR_RAIN_LIST: &str = "/Data/js/obs_img/Observe_radar_rain.js";
const OBSERVE_RADAR_RAIN_DIR: &str = "/Data/radar_rain/";

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    sat_img: Option<String>,
    #[arg(long)]
    radar_cloud: Option<String>,
    #[arg(long)]
    radar_rain: Option<String>,

    #[arg(default_value = "images", help = "download dir")]
    dir: String,

    #[arg(
        long,
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
        let url = Url::from_str(CWA_HOST)?.join(dir)?.join(&self.img)?;

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
struct Task<'a> {
    list: &'a str,
    dir: &'a str,
    contains: &'a str,
}

impl<'a> Task<'a> {
    fn new(list: &'a str, dir: &'a str, contains: &'a str) -> Self {
        Self {
            list,
            dir,
            contains,
        }
    }

    fn new_sat(contains: &'a str) -> Self {
        Self::new(OBSERVE_SAT_LIST, OBSERVE_SAT_DIR, contains)
    }

    fn new_radar(contains: &'a str) -> Self {
        Self::new(OBSERVE_RADAR_LIST, OBSERVE_RADAR_DIR, contains)
    }

    fn new_radar_rain(contains: &'a str) -> Self {
        Self::new(OBSERVE_RADAR_RAIN_LIST, OBSERVE_RADAR_RAIN_DIR, contains)
    }

    async fn download_list(
        &self,
        client: &mut reqwest::Client,
    ) -> Result<Vec<Img>, Box<dyn Error>> {
        let url = Url::from_str(CWA_HOST)?.join(&self.list)?;
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
    debug!("setup dir");
    let images_dir = Path::new(&args.dir);
    check_dir(images_dir).expect("can not create dir");

    // create task
    let mut tasks = Vec::new();

    if let Some(ref sat) = args.sat_img {
        tasks.push(Task::new_sat(sat));
    }

    if let Some(ref radar) = args.radar_cloud {
        tasks.push(Task::new_radar(radar));
    }

    if let Some(ref radar_rain) = args.radar_rain {
        tasks.push(Task::new_radar_rain(radar_rain));
    }

    let cycle_time = if args.interval != 0 {
        Duration::from_secs(args.interval)
    } else {
        Duration::from_secs(3600)
    };
    let mut interval = time::interval(cycle_time);

    loop {
        interval.tick().await;
        let result = run_tasks(&tasks, images_dir).await;

        match result {
            Ok(_) => info!("tasks finished"),
            Err(err) => error!("tasks error: {}", err),
        }

        if args.interval == 0 {
            break;
        }
    }

    info!("program exited");
}

fn check_dir(path: &Path) -> Result<(), std::io::Error> {
    if path.is_dir() {
        Ok(())
    } else {
        std::fs::create_dir_all(path)
    }
}

async fn run_tasks<'a>(tasks: &[Task<'a>], out_dir: &Path) -> Result<(), Box<dyn Error>> {
    info!("run tasks...");
    let mut client = Client::new();
    for task in tasks {
        let _ = do_task(&mut client, task, out_dir).await;
    }

    info!("end tasks...");
    Ok(())
}

async fn do_task<'a>(
    client: &mut Client,
    task: &Task<'a>,
    out_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    info!("download list");
    let list = task.download_list(client).await?;
    let target_imgs_iter = list.iter().filter(|x| x.img.contains(task.contains));

    for img in target_imgs_iter {
        let dest = out_dir.join(img.filename());
        if dest.is_file() {
            debug!("skiped {}", dest.to_str().unwrap());
            continue;
        }
        let response = img.download(client, &task.dir).await;
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

#[inline]
async fn save_file<R: AsyncRead + Unpin + ?Sized>(dest: &Path, reader: &mut R) -> io::Result<u64> {
    let mut file = File::create(dest).await?;

    let result = copy(reader, &mut file).await;

    match result {
        Ok(size) => {
            info!("saved {} {}", dest.to_str().unwrap(), human_size(size));
        }
        Err(ref err) => {
            warn!("cannot save file {}", err);
            let _ = remove_file(dest).await;
        }
    }

    result
}

#[inline]
fn human_size(size: u64) -> String {
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