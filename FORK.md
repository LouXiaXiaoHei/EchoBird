# EchoBird fork — 本仓库说明

这是 [edison7009/EchoBird](https://github.com/edison7009/EchoBird) 的个人自用 fork
（上游仓库：<https://github.com/edison7009/EchoBird>）。除了上游功能，只加了下面这些
"自用 + 自发布"需要的改动，方便自己出安装包、自己收更新。

- 与上游的差异：本文档
- **每个版本改了什么**：[CHANGELOG.md](CHANGELOG.md)
- 本仓库的 release 均为**未签名预览版**，版本号形如 `x.y.z-lxxh-preview.N`
- 产品本身、公告与商业询价，请看上游 [echobird.ai](https://echobird.ai)

## 相对上游改了哪些

| 改动 | 位置 |
| --- | --- |
| 设置 → **网络**：全局出站代理（HTTP / HTTPS / SOCKS5，可带账号密码） | `src/components/SettingsDialog.tsx`、`src-tauri/src/services/network_proxy.rs` |
| 模型管理新增 **Command Code** 供应商，并让本地新增供应商在联网时也不被远端目录覆盖 | `src/data/modelDirectory.json`、`src/pages/ModelNexus/ModelNexus.tsx` |
| **OpenCode Go** 的 API Key 请求补齐 `x-opencode-session` 系列头（中转 + 直连聊天 + 模型测试） | `src-tauri/src/services/smart_router/zen_free.rs`、`llm_client.rs`、`model_manager.rs` |
| **Command Code** 用量查询（走 `/alpha/billing` 路由，5 小时 / 每周 / 月度） | `src-tauri/src/services/usage_providers/commandcode.rs` |
| 用量请求统一带 `User-Agent`（否则 Cloudflare 返回 `403 error code: 1010`） | `src-tauri/src/services/usage_providers/mod.rs` |
| 窗口配额文案补「剩余」标签，避免 `100% 4h59m` 被读成已用满 | `src/components/ModelQuotaSummary.tsx` |
| 中文 / 日文 / 韩文输入法选词时按回车不再误发送 | `src/pages/MotherAgent/MotherAgentMain.tsx` |
| 版本检查 / 更新日志 / 应用内更新全部指向**本仓库**的 release | `src/config/release.ts`、`src-tauri/src/services/self_update.rs` |
| macOS 也能在应用内更新（下载 dmg 并交给 Finder） | `src-tauri/src/services/self_update.rs` |
| 去掉 release 流程里上游私有的 Antigravity OAuth 硬门禁（缺 secret 也能构建） | `.github/workflows/release.yml` |
| 发布版本 manifest 为 release 资产 `index.json` | `.github/workflows/sync-version-manifest.yml` |
| 每天自动合并上游 `main`，冲突时开 issue | `.github/workflows/sync-upstream.yml` |

版本检查会按顺序尝试三个源，任一可用即可：

1. `releases/latest/download/index.json`（随发布一起上传，最新）
2. `cdn.jsdelivr.net/gh/LouXiaXiaoHei/EchoBird@main/docs/api/version/index.json`（国内通常可达）
3. `raw.githubusercontent.com/...`（权威分支副本）

## 版本号约定

可以用带后缀的版本号，比如 `6.0.7-lxxh-preview`（`src-tauri/tauri.conf.json` 的
`version` 与 git tag 保持一致即可，`v` 前缀只加在 tag 上）。注意三点：

1. **核心数字要往上走**。semver 里 `6.0.6-lxxh-preview` 比 `6.0.6` **旧**，
   所以装了上游 6.0.6 的人不会收到更新提示；想让他们看到提示，得写
   `6.0.7-lxxh-preview` 这类"数字更大 + 后缀"的形式。
   同一核心号的后缀按 semver 排序：`-preview` < `-preview.2` < `-preview.10`。
   `isNewerVersion`（`src/utils/version.ts`）已按 semver 实现并有单测覆盖。
2. **Windows 只打 NSIS，不出 MSI**。WiX 对 MSI 的 ProductVersion 要求严格数字，
   带后缀会打包失败；NSIS 会把 `VIProductVersion` 强制成数字，后缀不影响。
   应用内更新也只用 `-setup.exe`。
3. **macOS 尽量别用后缀**。dmg 里的 `CFBundleShortVersionString` /
   `CFBundleVersion` 按 Apple 规范应是数字，带后缀在个别系统上可能出现
   "应用已损坏/无法打开"。如果打了带后缀的包发现 mac 打不开，把 `version`
   换成纯数字重新出包即可（tag 仍可保留后缀）。

## 发版流程

1. 改 `src-tauri/tauri.conf.json` 的 `version`（例：`6.0.6` → `6.1.0`），同时把版本号写进 `docs/api/version/index.json` 的 `version` 与 `releaseNotesI18n`（发版日志）。
2. 提交并推送 `main`。
3. 打 tag 并推送：`git tag v6.1.0 && git push origin v6.1.0`
   —— `release.yml` 会创建**草稿** release，并矩阵构建
   Windows x64、macOS arm64 / x64、Linux x64 / arm64，然后统一重命名资产为
   `EchoBird_<版本>_<平台>.<后缀>`。
4. 构建完成后在 Releases 页面确认资产齐全，再把草稿**发布**。
5. 手动运行 **Sync Version Manifest**（Actions → Run workflow，填 `v6.1.0`）：
   刷新 manifest、上传 `index.json` 到该 release、同步 release 说明。
   这一步才是真正让老版本 App 提示"有新版"的动作，建议先自己装一遍测试包。

## macOS 安装注意

构建出的 dmg **未签名/未公证**，首次打开会被 Gatekeeper 拦下。
用户侧执行一次即可：

```bash
xattr -dr com.apple.quarantine /Applications/EchoBird.app
```

## 应用内更新怎么走

- Windows：下载 `EchoBird_<版本>_Windows_x64-setup.exe`，启动安装向导后本进程退出。
- macOS：下载 `EchoBird_<版本>_macOS_arm64.dmg`（Intel 为 `_x64`）到临时目录并用 Finder 打开，App 不退出，由用户拖进「应用程序」。
- Linux：应用内不处理，直接打开 release 页面。

## 密钥 / secrets

不需要任何私有 secret。上游的 Antigravity OAuth 凭据若未配置，
`antigravity_accounts/quota.rs` 会退回公开默认值（编译期 `option_env!`）。

## 上游同步

`sync-upstream.yml` 每天 03:17 UTC（也可手动触发）把 `edison7009/EchoBird@main`
以 merge 方式并入本仓库；有冲突时**不推送**，改为开一个 issue 提示手工解决。
