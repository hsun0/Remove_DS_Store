# rmds

> **rmds 0.1.1 — 安全預覽及移除 ZIP 與資料夾中的 macOS metadata。**

`rmds` 精確辨識 `.DS_Store`、AppleDouble `._*` 與 `__MACOSX` metadata。ZIP 模式建立安全的清理副本；folder 模式預設只預覽，只有明確使用 `--apply` 並完成互動確認後才會原地刪除。

`rmds` 是以 Rust 開發的跨平台 CLI。它清的是 macOS 產生的 metadata，但工具本身以 macOS、Windows 與 Linux 都能執行為目標。

## 為什麼 ZIP 裡會有這些檔案？

macOS 在建立或封裝檔案時可能加入：

- `.DS_Store`：Finder 儲存資料夾顯示設定的檔案。
- `._*`：AppleDouble sidecar，保存 resource fork 或 extended attributes。
- `__MACOSX/`：部分 macOS 壓縮工具集中放置 AppleDouble metadata 的資料夾。

這些檔案對一般專案或 ZIP 內容通常沒有用途，在 Windows、Linux 或公開發佈的 archive 中尤其容易造成干擾。`rmds` 只處理精確符合規則且檔案型別符合預期的項目；`.gitignore`、`.env`、`.hidden`、`.DS_Store.backup`、`foo._bar` 與 `__MACOSX-file.txt` 都會保留。

## 從 repository 安裝

目前先以 clone repository 後從原始碼安裝為主。需要先安裝 [rustup](https://rustup.rs/)；repository 已用 `rust-toolchain.toml` 固定 Rust **1.97.1**，rustup 會取得相同的 compiler、rustfmt 與 Clippy。

```bash
git clone <repository-url>
cd Remove_DS_Store
rustup show
cargo test --locked
cargo install --path . --locked
```

`cargo install` 會建立最佳化版本，並將 `rmds` 安裝到 Cargo 的 executable 目錄：

```text
macOS / Linux: $HOME/.cargo/bin/rmds
Windows:       %USERPROFILE%\.cargo\bin\rmds.exe
```

如果使用者有設定自訂的 `CARGO_HOME`，實際位置會是 `$CARGO_HOME/bin`。安裝完成後先嘗試：

```bash
rmds --help
```

若 shell 顯示找不到 `rmds`，依下方環境設定 PATH。

### macOS：zsh

目前的 terminal session：

```zsh
export PATH="$HOME/.cargo/bin:$PATH"
rehash
rmds --help
```

若要永久生效，將這一行加入 `~/.zshrc`：

```zsh
export PATH="$HOME/.cargo/bin:$PATH"
```

重新開啟 terminal，或立即載入：

```zsh
source "$HOME/.zshrc"
```

### Linux：bash

目前的 shell session：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
hash -r
rmds --help
```

若要永久生效，將這一行加入 `~/.bashrc`：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

立即載入：

```bash
source "$HOME/.bashrc"
```

部分 Linux login shell 使用 `~/.profile` 或 `~/.bash_profile`；若重新登入後設定消失，請將相同 PATH 設定加入該 login profile。

### macOS／Linux：fish

`fish_add_path` 會將設定保存在 fish 的 universal variables：

```fish
fish_add_path $HOME/.cargo/bin
rmds --help
```

### Windows：PowerShell

只套用到目前的 PowerShell session：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Get-Command rmds
rmds.exe --help
```

若要永久生效，開啟 Windows「環境變數」，在使用者的 `Path` 新增：

```text
C:\Users\<使用者名稱>\.cargo\bin
```

儲存後關閉並重新開啟 PowerShell 或 Windows Terminal。

### Windows：Command Prompt

只套用到目前的 CMD session：

```bat
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where rmds
rmds.exe --help
```

永久設定同樣使用 Windows「環境變數」介面，將 `%USERPROFILE%\.cargo\bin` 加入使用者的 `Path`。

### 不安裝到 PATH，直接執行

也可以只建立 repository 內的 executable：

```bash
cargo build --release --locked
```

接著執行：

```text
./target/release/rmds          # macOS / Linux
.\target\release\rmds.exe     # Windows PowerShell
```

## 使用方式

### 預覽資料夾

folder command 會遞迴掃描；預設是安全的 preview，不會修改 filesystem：

```bash
# 預覽目前資料夾，等同 rmds folder .
rmds folder

# 預覽指定資料夾
rmds folder ./project
```

輸出會顯示解析後的 absolute target、依相對路徑穩定排序的候選項目，以及 apply 指令。Preview 不會刪除或修改檔案，也不會建立 token、cache、lock、設定或其他狀態檔。

### 套用資料夾清理

真正刪除必須使用唯一的明確形式：

```bash
rmds folder --apply ./project
```

Apply 必須明確指定 path；`rmds folder --apply` 與 `rmds folder ./project --apply` 都會被拒絕。即使目標是目前資料夾，也要輸入：

```bash
rmds folder --apply .
```

Apply 不會沿用之前的 preview。它會重新掃描、再次完整列出候選項目，並警告這是無法復原的 in-place operation。只有在真正的互動式 terminal 中輸入完全相同的 `DELETE` 才會開始刪除；大小寫不同、前後空白、空輸入都會取消。pipe 或 redirected confirmation 會被拒絕，因此以下指令不會刪除：

```bash
echo DELETE | rmds folder --apply ./project
```

找不到 metadata 時會正常結束，不詢問確認。folder cleanup 不提供 rollback；若刪除途中因權限、I/O 或 filesystem race 失敗，`rmds` 會立即停止、回傳失敗狀態並列出已成功刪除的項目。

### Folder metadata 規則

- `.DS_Store`：只刪除名稱完全相同的 regular file 或 symbolic link。真正的同名 directory 會保留，但仍掃描其內容。
- `._*`：只刪除 filename 以 `._` 開頭的 regular file 或 symbolic link。符合名稱的真正 directory 會保留，但仍掃描其內容。
- `__MACOSX`：只有名稱完全相同的真正 directory 會整棵視為一個候選項目；同名 regular file 會保留。
- metadata symbolic link 只移除 link，不會觸碰 target；其他 directory symlink 不會被進入或修改，broken symlink 也不會被 follow。
- Preview 與 Apply 都拒絕 symbolic-link root 與 filesystem root（例如 `/` 或 `C:\`）。

### 清理 ZIP

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
rmds --version
rmds folder --help
rmds zip --help
```

查詢目前安裝的版本：

```text
$ rmds --version
rmds 0.1.1
```

## 安全保證

### Folder mode

- Preview 是預設行為；完整掃描與顯示完成後直接結束，filesystem 維持不變。
- Apply 需要明確 path、重新掃描、in-place 危險警告、完整候選清單、interactive terminal 與精確 `DELETE`。
- 掃描使用 host filesystem path semantics 與 iterative traversal，不依賴 UTF-8，也不 follow symbolic links。
- 刪除前會重新驗證 root、候選名稱、型別與父資料夾，檢查項目仍位於 validated root 內；發現變動就停止。
- folder deletion 並非 atomic。部分失敗時不宣稱或嘗試 rollback，會誠實回報已刪除項目。

### ZIP mode

- 永不修改來源 ZIP，也不提供 `--in-place` 或 `--force`。
- 輸出已存在時立即失敗，絕不覆寫。
- 先寫入輸出資料夾內的安全暫存檔；所有 entry、CRC、flush 都成功後才以 no-clobber 方式發布最終檔。
- 失敗時不留下看似成功的 final output，暫存檔由 RAII cleanup 清除。
- ZIP-to-ZIP 處理，不 extract，不會依 entry path 讀寫檔案系統其他位置。
- 拒絕 absolute、traversal、NUL、backslash、Windows drive prefix 與重疊 entry。
- 每個 entry 以固定 64 KiB buffer 串流驗證；RAM 不隨 archive 或單一 entry 大小線性增加。
- 一般保留項目使用 raw copy，維持 filename bytes、compression method、compressed data、timestamp、Unix permissions、executable bit、directory、extra fields 與 entry comment。Symlink 以專用 writer 保留連結語意與 target；無法安全表示的非 UTF-8 symlink 會失敗。

目前支援完整驗證 Stored 與 Deflate entry。遇到 encrypted entry 或未啟用的 compression method 會保守失敗，不產生 final output。ZIP round-trip 的已知 metadata 邊界請見 [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md)。

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

## 尚未實作

0.1.1 沒有 `--yes`、`--force`、`--no-confirm`、`--dry-run`、trash／recycle-bin、rollback、Git cleaning、`.gitignore` 修改、GUI、設定檔、telemetry、套件管理器發佈、release automation 或自動更新。Preview 已是 folder command 的預設行為，因此不提供 `--dry-run` alias。

## License

[MIT](LICENSE)。MIT、Apache-2.0 與雙授權的簡短比較及 dependency 選型記錄見 [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md)。
