# Quakewatch

台灣地震速報的獨立 Rust 專案，包含 GPUI 桌面版與無桌面環境可用的 CLI。不是地牛官方版，不使用地牛的程式碼、素材、私有伺服器或 P2P 協定。**不可用作唯一的防災訊號。**

## GPUI 桌面版

首版提供臺灣底圖、震央與 P／S 波示意、預估震度／到時、所在地與門檻、最近通知、即時監測和離線演練。預設不連網，按 `D` 演練、`L` 開關監測，離線時按 `C` 換所在地、`T` 換門檻。

Linux 預覽需要 X11 或 XWayland、Vulkan 驅動及繁中字型（例如 Noto Sans CJK）。在專案根目錄建置：

```sh
sudo apt install pkgconf libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libfontconfig1-dev libvulkan1 mesa-vulkan-drivers fonts-noto-cjk
cargo run --manifest-path desktop/Cargo.toml --locked -- --demo
```

底圖取自 [Natural Earth 1:50m v5.1.2](https://github.com/nvkelso/natural-earth-vector/tree/v5.1.2)，屬公有領域。首版桌面版不是原版的完整替代；未提供開機自啟、自動更新、地震報告列表或原版連動參數。CLI 的 JSON hook 是另一套介面。

## CLI

```sh
cargo build --release --locked
target/release/quakewatch replay --lat 25.04 --lon 121.57 --min 2 \
  --at-ms 1700000005000 fixtures/demo.sse
```

回放檔是人造演練，固定時鐘；會依序輸出 `alert`、`update`、`cancel`。不連外、不執行 hook。讀 stdin 時，檔名使用 `-`。

```sh
# 先閱讀資料提供者的條款；僅在已獲授權的用途使用。
target/release/quakewatch live --lat 25.04 --lon 121.57 --check
target/release/quakewatch live --lat 25.04 --lon 121.57 --min 2
```

- `--check` 連到實際 HTTPS SSE，驗證第一份 JSON 快照就退出；空陣列表示目前沒有活動速報，不代表驗證過真實地震的通知。
- 可用 `--url` 指定已授權的同格式 HTTPS 來源，`--backup` 可選；斷線重連，重連保留去重狀態。需讓系統時鐘同步；事件在 180 秒後視為過期。只處理 `cwa` 來源。
- 標準輸出每行一筆 JSON：`alert`、`update`、`below_threshold` 或 `cancel`，診斷走 stderr。空快照不視為撤報。狀態只保存在記憶體，重啟後可重複通知。
- `--hook /absolute/path/to/program` 可選，Linux 上將同一筆 JSON 送至程式的 stdin（不經 shell、單一背景 worker、每個直接子程序最多 5 秒；佇列滿只略過 hook，stdout 仍輸出）。不等同地牛 v4.2.0 的連動參數。不要接危險設備或公開轉發警報。
- 震度是公開的 PGA 經驗式粗估；5 弱以上不再細分。到時採震源直線距離與固定 S 波速 3.5 km/s，不含場址效應或分層模型；沒有地圖、桌面通知、音效、報告列表。

## 調查與來源

- [地牛官網](https://eew.earthquake.tw/)：最新列出 **v4.2.0（2026/07/26）**，恢復連動參數；同頁軟體使用條款第三條明文禁止反編譯，所以未下載或拆解原版。參考 [公開連動範例](https://github.com/chemars/EEW_Extension) 確認功能方向，未複用其程式碼。
- [ExpTech DPIP API 文件](https://github.com/ExpTechTW/DPIP/blob/main/api.md)：`/api/v2/eq/eew?sse=1` 非壓縮 SSE，`event: info` 是連線資訊，普通 `data` 訊息是 JSON 快照。欄位由歷史 EEW 端點實際回應核對。供應商 [使用條款](https://exptechtw.github.io/api-docs/docs/api/start/tos/) 對改作、轉發等有規定；這裡不附真實封包、不提供資料轉播服務。
- 估算式參考 [ExpTechTW/TW-EEW 公開說明](https://github.com/ExpTechTW/TW-EEW#原理)；低震度 PGA 門檻參考[中央氣象署地震震度分級表](https://opendata.cwa.gov.tw/opendatadoc/Earthquake/E-A0015-001.pdf)。此原型沒有反推出地牛的模型，數值不保證與地牛相同。
