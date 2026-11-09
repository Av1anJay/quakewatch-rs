# Quakewatch

台灣地震速報的獨立 Rust 專案，包含 GPUI 桌面版與無桌面環境可用的 CLI。不是地牛官方版，不使用地牛的程式碼、素材、私有伺服器或 P2P 協定。**不可用作唯一的防災訊號。**

## GPUI 桌面版

首版提供臺灣底圖、震央與 P／S 波示意、預估震度／到時、所在地與門檻、最近通知、即時監測和離線演練。預設不連網，按 `D` 演練、`L` 開關監測，離線時按 `C` 換所在地、`T` 換門檻。

Linux 預覽需要 X11 或 XWayland、Vulkan 驅動及繁中字型（例如 Noto Sans CJK）。在專案根目錄建置：

```sh
sudo apt install pkgconf libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libfontconfig1-dev libvulkan1 mesa-vulkan-drivers fonts-noto-cjk
cargo run --manifest-path desktop/Cargo.toml --locked -- --demo
```

底圖取自 [Natural Earth 1:50m v5.1.2](https://github.com/nvkelso/natural-earth-vector/tree/v5.1.2)，屬公有領域。

### v0.3.0 設定與連動

按 `S` 開啟設定，`1`～`4` 切換「強震即時警報／地震報告／其他／關於」，`Enter` 儲存、`Esc` 取消。

- 設定可保存所在地、預警門檻、75／100／125／150% 縮放和連動選項。放大受視窗寬度限制；縮放主要影響字級與 rem 間距，不是整個視窗的等比例縮放。自訂經緯度仍使用命令列。主畫面 `C`／`T` 是本次暫時變更，要持久化需在設定儲存。
- 「其他」的 `B` 選擇本機程式，`T` 測試。Linux 的檔案選擇器需要運作中的 `xdg-desktop-portal` 及桌面後端。Windows 接受 `.exe`、`.bat`、`.cmd`；macOS／Linux 接受有執行權限的執行檔或腳本，不是 `.app` 目錄。
- 真實速報達所在地門檻時，傳入兩個位置參數：`震度 抵達秒數`，例如 `4 10`；排隊的等待時間會從抵達秒數扣除，超過 180 秒的佇列工作略過。「僅執行一次」阻止本次啟動期間同一事件的更正報重複執行，最多保留 4096 個事件 ID，達上限時停止新增單次連動。狀態不跨重新啟動保存，滿佇列會略過連動。演練不觸發連動，「測試」則會真的執行選定程式。
- **不完整的震度相容性：**目前模型只分到 ≥5弱，高震度統一傳 `5-`，不會正確區分 `5+`、`6-`、`6+`、`7`，不能當作地牛的數值等價替代。連動程式最多執行 10 秒，只管理直接子程序，不接危險設備。
- 「地震報告」的 `R` 手動讀取最近 10 筆已發布報告，與即時預警分開。報告含地點、規模、深度和最大實測震度；尚無測站地圖或報告詳情。

### 跨平台發佈

[Releases](https://github.com/Av1anJay/quakewatch-rs/releases) 提供 Windows x86_64、Linux x86_64、macOS Apple Silicon／Intel 原生壓縮包及 `SHA256SUMS`。解壓後執行 `quakewatch-gui`（Windows 為 `.exe`）。檔案未經 Windows 簽章或 Apple notarization；macOS 目前是命令列執行檔，不是 `.app`／DMG。Linux 以 Ubuntu 24.04 建置，需要相容的 glibc、X11、Vulkan 與系統字型。CI 測試四個平台並在 Linux 用虛擬螢幕做開窗驗證；不等於 Windows／macOS 桌面互動已驗證。

推送 `v*` tag 後，GitHub Actions 會在四組平台測試、編譯、打包；全部成功才發佈預覽版。

### 與地牛的功能差距

**這仍是預覽版，不是功能完全相同的地牛替代品。**尚缺開機自啟、自動下載與安裝更新、系統匣／背景常駐、系統通知、警報音效／語音、完整高震度估算、細部行政區選擇、測站與報告地圖操作。沒有地牛的 P2P、私有服務或演算法。這些未完成項不會以假的勾選框冒充功能。

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
