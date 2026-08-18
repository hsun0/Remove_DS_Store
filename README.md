# rmds

> **rmds — Remove unwanted macOS metadata from ZIP archives.**

直接清除 ZIP 裡的 `.DS_Store`、AppleDouble `._*` 與 `__MACOSX` metadata，不把 archive 解壓到檔案系統。

`rmds` 是以 Rust 開發的跨平台 CLI。它清的是 macOS 產生的 metadata，但工具本身以 macOS、Windows 與 Linux 都能執行為目標。

## 為什麼 ZIP 裡會有這些檔案？

macOS 在建立或封裝檔案時可能加入：

- `.DS_Store`：Finder 儲存資料夾顯示設定的檔案。
- `._*`：AppleDouble sidecar，保存 resource fork 或 extended attributes。
- `__MACOSX/`：部分 macOS 壓縮工具集中放置 AppleDouble metadata 的資料夾。

這些檔案對一般 ZIP 內容通常沒有用途，在 Windows、Linux 或公開發佈的 archive 中尤其容易造成干擾。`rmds` 只移除精確符合規則的 path component；`.gitignore`、`.env`、`.hidden`、`foo._bar` 與 `__MACOSX-file.txt` 都會保留。

## 安裝

正式 release 可直接下載平台對應的單一 executable：

```text
macOS / Linux: rmds
Windows:       rmds.exe
```

第一階段 repository 尚未加入 release automation。現在可依下方「從原始碼建置」產生 executable；執行完成的程式不需要 Python、Node.js、Java、Rust、Cargo、Homebrew、libzip 或系統 zlib。

## 使用方式

```bash
rmds zip photos.zip
```

預設建立 `photos-clean.zip`，來源 `photos.zip` 保持不變。

### 自訂輸出

```bash
rmds zip photos.zip -o clean.zip
```

說明：

```bash
rmds --help
rmds zip --help
```

## 安全保證

- 永不修改來源 ZIP，也不提供 `--in-place` 或 `--force`。
- 輸出已存在時立即失敗，絕不覆寫。
- 先寫入輸出資料夾內的安全暫存檔；所有 entry、CRC、flush 都成功後才以 no-clobber 方式發布最終檔。
- 失敗時不留下看似成功的 final output，暫存檔由 RAII cleanup 清除。
- ZIP-to-ZIP 處理，不 extract，不會依 entry path 讀寫檔案系統其他位置。
- 拒絕 absolute、traversal、NUL、backslash、Windows drive prefix 與重疊 entry。
- 每個 entry 以固定 64 KiB buffer 串流驗證；RAM 不隨 archive 或單一 entry 大小線性增加。
- 一般保留項目使用 raw copy，維持 filename bytes、compression method、compressed data、timestamp、Unix permissions、executable bit、directory、extra fields 與 entry comment。Symlink 以專用 writer 保留連結語意與 target；無法安全表示的非 UTF-8 symlink 會失敗。

目前支援完整驗證 Stored 與 Deflate entry。遇到 encrypted entry 或未啟用的 compression method 會保守失敗，不產生 final output。ZIP round-trip 的已知 metadata 邊界請見 [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md)。

## 從原始碼建置

需要先安裝 [rustup](https://rustup.rs/)。Repository 已用 `rust-toolchain.toml` 固定 Rust **1.97.1**；rustup 會依檔案取得相同的 compiler、rustfmt 與 Clippy。

```bash
git clone <repository-url>
cd rmds
rustup show
cargo build --release --locked
```

Executable 位於：

```text
target/release/rmds      # macOS / Linux
target/release/rmds.exe  # Windows
```

## 開發與測試

```bash
cargo fmt --check
cargo build --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

`Cargo.toml`、`Cargo.lock` 與 `rust-toolchain.toml` 都必須提交。Dependency 與 compiler 升級是明確的 repository change：review lockfile diff、執行完整檢查，再由 macOS／Windows／Linux CI 驗證。

## 跨平台目標

GitHub Actions 在 `ubuntu-latest`、`macos-latest`、`windows-latest` 執行相同 locked build、test、format 與 Clippy。程式不呼叫 Finder、Apple-only API、shell、`/usr/bin/zip` 或 `/usr/bin/ditto`。

## Roadmap

未來階段可能評估 `rmds repo`，用於 Git tracked metadata 與 `.gitignore`。第一階段不實作 Git integration、directory cleaning、`--dry-run`、GUI、套件管理器或自動更新。

## License

[MIT](LICENSE)。MIT、Apache-2.0 與雙授權的簡短比較及 dependency 選型記錄見 [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md)。
