# CWA Images

Rust 新手打造的氣象署圖片爬蟲

## 功能

可自訂間隔(單位: 秒)循環任務

當然你想用 cron job 我也不阻止你

支援的類型:
- [x] 衛星影像
- [x] 雷達
- [x] 降雨雷達

## 安裝

需要 rust toolchain

```sh
git clone https://github.com/Benny1923/cwa-images
cargo install --path cwa-images
```

~~或者是等我會用 github actions 時(when?)從Releases下載~~

## 參數

```
$ cwa_images -h 
Usage: cwa_images.exe [OPTIONS] [DIR]

Arguments:
  [DIR]  [default: images]

Options:
      --sat-img <SAT_IMG>
      --radar-cloud <RADAR_CLOUD>  
      --radar-rain <RADAR_RAIN>
      --interval <INTERVAL>        [default: 0]
  -d, --debug                      print debug message
  -h, --help                       Print help
```

## 版權聲明

本程式產生圖片資料版權為中央氣象署所有: [政府開放資料宣告](https://www.cwa.gov.tw/V8/C/information.html)
