# rmds

> **rmds 0.1.3 — 安全檢查、預覽及移除 ZIP、資料夾與 Git working tree 中的 macOS metadata。**

`rmds` 精確辨識 `.DS_Store`、AppleDouble `._*` 與 `__MACOSX` metadata。三種目標都有適合 CI 的唯讀 `--check`；ZIP 模式建立安全的清理副本；folder 模式預設只預覽，只有明確使用 `--apply` 並完成互動確認後才會原地刪除；repo 模式會先顯示 Git 狀態與警告，只有輸入精確的 `DELETE` 才清理 working tree。

`rmds` 是以 Rust 開發的跨平台 CLI。它清的是 macOS 產生的 metadata，但工具本身以 macOS、Windows 與 Linux 都能執行為目標。

## 為什麼會有這些檔案？

macOS 在建立或封裝檔案時可能加入：

- `.DS_Store`：Finder 儲存資料夾顯示設定的檔案。
- `._*`：AppleDouble sidecar，保存 resource fork 或 extended attributes。
- `__MACOSX/`：部分 macOS 壓縮工具集中放置 AppleDouble metadata 的資料夾。

這些檔案對一般專案或 ZIP 內容通常沒有用途，在 Windows、Linux 或公開發佈的 archive 中尤其容易造成干擾。`rmds` 只處理精確符合規則且檔案型別符合預期的項目；`.gitignore`、`.env`、`.hidden`、`.DS_Store.backup`、`foo._bar` 與 `__MACOSX-file.txt` 都會保留。

## 從 repository 安裝

目前不提供預先編譯的下載檔或套件管理器安裝，主要使用方式是 clone repository 後在自己的電腦編譯安裝。

### 先備需求

安裝前需要：

- **Git**：用來 clone repository；使用 `rmds repo` 時也必須能從 `PATH` 執行 `git`。
- **rustup**：建議透過 [Rust 官方安裝頁面](https://rustup.rs/)安裝。rustup 會一併提供 `rustc` 與 Cargo，不需要另外安裝這兩個工具。
- **網路連線**：第一次建置時，rustup 需要下載指定的 Rust toolchain，Cargo 也需要下載 `Cargo.lock` 鎖定的 dependencies。
- **Windows MSVC 建置工具**：Windows 使用預設 MSVC Rust toolchain 時，需要 linker、Windows SDK 與相關 libraries。`rustup-init` 通常會提示自動安裝；也可以依照 [rustup 的 MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html)安裝 Visual Studio 的「Desktop development with C++」。

支援目標為 macOS、Windows 與 Linux。repository 透過 `rust-toolchain.toml` 固定 Rust **1.97.1**，並要求 `rustfmt` 與 Clippy；進入 repository 後，rustup 會自動選擇並在需要時安裝相同版本。

先確認基本工具可以執行：

```bash
git --version
rustup --version
cargo --version
rustc --version
```

如果 `rustup`、`cargo` 或 `rustc` 顯示找不到指令，請關閉並重新開啟 terminal，讓 rustup 的 PATH 設定生效，再重新檢查。

### Clone、驗證與安裝

```bash
git clone https://github.com/hsun0/Remove_DS_Store.git
cd Remove_DS_Store
rustup show
cargo test --locked
cargo install --path . --locked
```

各指令的用途：

- `git clone`：下載完整 repository。
- `rustup show`：顯示目前選用的 toolchain，並確認專案固定的 Rust 版本可用。
- `cargo test --locked`：使用 `Cargo.lock` 鎖定的 dependencies 執行測試；建議在安裝前先確認測試通過。
- `cargo install --path . --locked`：從目前原始碼建立最佳化的 executable，然後安裝到 Cargo 的 executable 目錄。

如果 `cargo install` 顯示專案的 `rust-toolchain.toml` 覆蓋預設 toolchain，這是正常提醒：代表本次建置使用專案固定的 Rust 1.97.1，而不是電腦的其他預設版本。

`cargo install` 會建立最佳化版本，並將 `rmds` 安裝到 Cargo 的 executable 目錄：

```text
macOS / Linux: $HOME/.cargo/bin/rmds
Windows:       %USERPROFILE%\.cargo\bin\rmds.exe
```

如果使用者有設定自訂的 `CARGO_HOME`，實際位置會是 `$CARGO_HOME/bin`。安裝完成後確認版本與 help：

```bash
rmds --version
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

### 唯讀 Check 模式

Folder、Git repository 與 ZIP 共用相同的 check 語意：完整掃描、列出候選並以 exit code 回報，過程不詢問 `DELETE`，也不刪除、建立或覆寫檔案。

```bash
# PATH 預設為目前資料夾
rmds folder --check [PATH]
rmds repo --check [PATH]

# ZIP 必須明確指定輸入檔
rmds zip --check archive.zip
```

Check mode 的 exit code：

- `0`：檢查完成，沒有 macOS metadata。
- `1`：檢查完成，找到至少一個 metadata；適合讓 CI job 判定內容不符合要求。
- `2`：參數或目標錯誤，或掃描／驗證無法完整完成。

### 資料夾清理

刪除必須使用唯一的明確形式：

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

### 清理 Git repository

repo mode 需要系統已安裝 Git，且 `git` 可以從 `PATH` 執行：

```bash
# 掃描目前 repository，顯示候選後詢問確認
rmds repo

# 指定 repository 或其內部子資料夾
rmds repo ./project
```

指定 repository 內的子資料夾時，`rmds` 會透過 read-only Git commands 解析並掃描完整 working-tree root。repo mode 沒有 `--apply`；command 會先完整顯示候選、Git classification 與風險警告，只有在 interactive terminal 輸入精確的 `DELETE` 才開始刪除。直接按 Enter、輸入其他文字或使用 pipe 都不會確認。

候選項目可能顯示：

- `[tracked]`：存在於 Git index，working tree 沒有額外修改。
- `[tracked, modified]`：tracked，但有尚未提交的修改。
- `[untracked]`：未被 Git 追蹤。
- `[ignored]`：符合 Git ignore 規則。
- `[mixed]`：`__MACOSX` tree 內含多種 Git 狀態。

### 清理 ZIP

```bash
rmds zip <input.zip> [-o <output.zip>]
```

### 其他指令

查詢目前安裝的版本：

```text
$ rmds --version
```

指令用法說明：
```text
$ rmds --help
```

## 安全保證

### Check mode

- `folder --check`、`repo --check` 與 `zip --check` 都是唯讀、非互動流程，不會呼叫 deletion function。
- 三種目標統一以 `0` 表示 clean、`1` 表示找到 metadata、`2` 表示無法完成檢查。
- Check 必須先完成整次掃描才回報結果；不會把 partial scan 誤報為通過。
- Check 可安全用於 CI、pipe 與 redirect；非互動輸出不含 ANSI color codes。

### Folder mode

- Preview 是預設行為；完整掃描與顯示完成後直接結束，filesystem 維持不變。
- Apply 需要明確 path、重新掃描、in-place 危險警告、完整候選清單、interactive terminal 與精確 `DELETE`。
- 掃描使用 host filesystem path semantics 與 iterative traversal，不依賴 UTF-8，也不 follow symbolic links。
- 刪除前會重新驗證 root、候選名稱、型別與父資料夾，檢查項目仍位於 validated root 內；發現變動就停止。
- folder deletion 並非 atomic。部分失敗時不宣稱或嘗試 rollback，會誠實回報已刪除項目。

### Repo mode

- 單一流程是 scan → display → warn → exact `DELETE` → revalidate → delete；confirmation 前不做任何 filesystem mutation。
- 所有 Git 查詢都透過 argument-safe、machine-readable command 執行，並設定 `GIT_OPTIONAL_LOCKS=0`；Git 不存在只會讓 repo mode 失敗。
- `.git` 是獨立 hard boundary；刪除前會再次檢查 candidate、parents、Git directory、common directory 與 nested repository boundary。
- `.gitignore` 永遠只提供文字建議，不會自動修改。
- Git 並非完整備份；untracked、ignored、uncommitted content 可能無法復原。

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

## License

[MIT](LICENSE)。MIT、Apache-2.0 與雙授權的簡短比較及 dependency 選型記錄見 [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md)。
