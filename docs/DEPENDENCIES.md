# 技術與依賴選型

本文件記錄 0.1.2 的選擇；更新 dependency 或 Rust toolchain 時應一併 review。

## Rust toolchain

- 固定 `1.97.1`（2026-07-16 的 stable 修正版），不使用浮動 `stable`。
- 採 Rust 2024 Edition；專案 `rust-version` 宣告為 `1.97`。
- `rust-toolchain.toml` 同時固定 `rustfmt` 與 `clippy`，`Cargo.lock` 固定完整 dependency graph。

## ZIP library

選擇 [`zip` 8.6.0](https://docs.rs/zip/8.6.0/zip/)；未採用 `rawzip`、`libzip` 或 `libarchive`。

| 評估項目 | 結論 |
|---|---|
| 成熟度與維護 | `zip-rs/zip2` 持續維護，具測試與 fuzzing，並依 PKWARE APPNOTE 實作。 |
| License | MIT，適合本專案。 |
| 跨平台 | Rust API，不依賴 Finder、Apple API 或外部 `zip` command；支援 macOS、Windows、Linux。 |
| Native/runtime dependency | 關閉 default features，只啟用 `deflate-flate2-zlib-rs`；此路徑使用 Rust 實作，不要求終端使用者安裝 libzip、zlib 或其他 runtime。 |
| Dependency tree | 功能縮減為 ZIP 核心、CRC 與 Deflate；不納入 AES、Bzip2、XZ、Zstd 等目前不需要的 codec/crypto tree。 |
| Streaming／大型 archive | entry 以固定 64 KiB buffer 驗證，不把 archive 或大型 entry 全部載入 RAM。ZIP writer 直接寫檔。 |
| ZIP64 | reader/writer 支援 ZIP64，writer 依門檻自動產生 ZIP64 結構。 |
| Compression | Stored 與 Deflate 可完整驗證；保留項目用 `raw_copy_file` 複製既有 compressed bytes，不重新壓縮、不改 method。其他 method 會保守失敗並刪除暫存輸出。 |
| Timestamp／permissions／symlink | 一般 entry 的 raw copy 保留 header metadata、Unix mode 與 executable bit。Symlink 用 crate 的專用 writer 重建 target 與 mode；無法安全表示的非 UTF-8 symlink 會失敗。 |
| Unicode | 規則判斷使用 raw filename bytes；raw copy 保留原始 filename encoding 與 UTF-8 flag，不做 Unicode normalization。顯示訊息則使用 crate 的 decoded name。 |
| Extra fields／comments | raw copy 保留 entry extra fields 與 entry comment；另外保留 archive raw comment。 |
| Corruption | 建檔前逐 entry 解壓至小 buffer 並讀到 EOF，以觸發結構、codec 與 CRC 驗證；重疊 entry 直接拒絕。 |
| Encryption | 第一階段沒有密碼介面，encrypted entry 會清楚失敗，不會產生可能未驗證的輸出。 |
| Security history | 選型時未在 RustSec advisory database 找到影響 `zip 8.6.0` 的未修補公告；仍應在有意識的 dependency update 流程中重新稽核。 |
| Raw preservation limits | 不保證保留 ZIP 前置 self-extracting stub、central-directory 數位簽章或 ZIP64 extensible data sector。重建 symlink 時可能不保留其 entry comment／extra fields／原 compression method。資料與 symlink semantics 優先；不為 bit-perfect archive round-trip 自行實作 ZIP。 |

`rawzip` 提供低階、stream-friendly ZIP 結構 API，但要求呼叫端自行組合 compression，且第一階段要自行承擔更多 metadata round-trip 細節。系統 `libzip`／`libarchive` 方案成熟，但會增加 native linking、不同發行版與 Windows 發佈複雜度。對 download-and-run 的小型 CLI，`zip` 的維護狀態與 metadata-aware raw copy 較合適。

## CLI parsing

目前語法包含 `rmds zip INPUT [-o OUTPUT]`、`rmds folder [PATH]`、`rmds folder --apply PATH` 與 `rmds repo [PATH]`。`std::env::args_os` 足以保留非 UTF-8 host path、提供可預測 validation，且不需加入 `clap` 的 derive 與 transitive dependencies。Folder／repository traversal、symlink 判斷及 interactive-terminal 檢查也都由 standard library 提供，因此 0.1.2 沒有新增 Rust dependency。若未來 subcommand 與 option 顯著增加，再重新評估 `clap`。

## Git executable

Repo mode 使用使用者 `PATH` 中的 Git executable，透過 `std::process::Command` 直接傳入 arguments，不經 shell。選擇系統 Git 而非 `git2`／`libgit2`，可沿用 Git 自己對 worktree、common directory、ignore rules 與 index 的語意，也避免 native library、transitive dependency 與跨平台 linking 負擔。

Repo mode 只呼叫 `rev-parse`、`ls-files` 與 `diff` 等 read-only 查詢，並設定 `GIT_OPTIONAL_LOCKS=0`，不執行 `add`、`rm`、`clean`、`reset`、`restore`、`checkout`、`stash`、`commit` 或 config mutation。filesystem traversal 明確排除 `.git`、nested repository 與 submodule boundary；`.gitignore` 只顯示建議，不自動修改。Git 不存在時只有 repo mode 失敗，ZIP 與 folder mode 不受影響。

## Atomic output

選擇 [`tempfile` 3.27.0](https://docs.rs/tempfile/3.27.0/tempfile/) 而非自行拼湊暫存檔名。它成熟、MIT OR Apache-2.0、支援主要桌面 OS，並提供 `persist_noclobber`，可在同一 filesystem 上避免覆寫競態。其平台 syscall bindings 編入 executable，不要求終端使用者安裝額外 runtime。

## License decision

- **MIT**：最簡短、寬鬆，保留 copyright/license notice 即可。
- **Apache-2.0**：同樣寬鬆，另有明確 patent grant 與 NOTICE 條款，文字較長。
- **MIT OR Apache-2.0**：Rust ecosystem 常見雙授權，使用者可選其一，但 repository 需維護兩份 license text。

第一階段採 **MIT**：對小型 CLI 與公開 contribution 足夠簡單；若未來 contributor 或組織需要明確 patent 條款，可再經 repository change 改為雙授權。
