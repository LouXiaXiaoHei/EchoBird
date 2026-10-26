# 版本改动记录

本仓库是 [edison7009/EchoBird](https://github.com/edison7009/EchoBird) 的个人自用
fork。与上游的整体差异见 [FORK.md](FORK.md)；这里按版本记录**每个发布改了什么**。

版本号形如 `x.y.z-lxxh-preview.N`，`N` 省略即 `.0`（semver 里
`-preview` < `-preview.1` < `-preview.2`）。上游功能请对照上游的 release notes。

---

## 6.0.6-lxxh-preview.2 — 2026-10-25

**修复**

- **Command Code 用量查询**：新增用量 provider。`/provider/v1` 没有用量接口
  （任何 `/provider/v1/usage` 之类路径都返回 404），因此改为用同一个 API Key 请求
  官方 CLI `/usage` 命令所用的 `/alpha/billing/credits` 与
  `/alpha/billing/subscriptions`，输出 5 小时窗口、每周窗口的已用百分比，并在套餐
  已知时补上月度额度。「刷新用量」不再以"不支持"静默失败。
  （`src-tauri/src/services/usage_providers/commandcode.rs`）
- **Cloudflare 请求头**：所有用量请求现在都带 `User-Agent`。Cloudflare 前置的接口
  （`api.commandcode.ai`、`opencode.ai`）对不带 UA 的请求直接返回
  `403 error code: 1010`，此前会被当成鉴权失败上报。
  （`src-tauri/src/services/usage_providers/mod.rs`）
- **配额显示歧义**：模型卡片上的窗口配额补上「剩余」标签。窗口型额度内部存的是
  **已用百分比**、界面显示的是**剩余**，此前只有 `100% 4h59m(100%)` 这样的裸数字，
  容易被读成"5 小时已用掉 100%"。现在显示为 `剩余 100% 4h59m(100%)`，与悬停提示
  的措辞一致。（`src/components/ModelQuotaSummary.tsx`）

**构建**

- 单平台的 release job 失败后不再跳过资产重命名，避免留下半成品 release。
  （`.github/workflows/release.yml`）

## 6.0.6-lxxh-preview.1 — 2026-10-25

**修复**

- **OpenCode Go 直连路径**：Agent 聊天与模型管理里的「测试」现在也会带上
  `x-opencode-session` 系列头。之前只有中转（smart router）路径加了头，直连仍被上游
  拒绝，报 `Request is missing x-opencode-session`。
  （`src-tauri/src/services/llm_client.rs`、`src-tauri/src/services/model_manager.rs`、
  `src-tauri/src/services/smart_router/zen_free.rs`）
- 模型测试失败时把状态码与响应体写入日志，便于定位（不再只有一句"测试失败"）。

## 6.0.6-lxxh-preview — 2026-10-25

首个自用预览版，基于上游 6.0.6：

- **设置 → 网络**：全局出站代理，支持 HTTP / HTTPS / SOCKS5（可带账号密码），
  EchoBird 启动的 Agent CLI、npm、git 等子进程一并生效，本机地址自动排除。
- **模型管理**：新增 Command Code 供应商；联网时本地新增的供应商不再被远端目录覆盖。
- **OpenCode Go**：API Key 请求补齐 `x-opencode-session` 系列头。
- **输入框**：中文 / 日文 / 韩文输入法选词时按回车不再误发送消息。
- **版本检查与更新**：全部跟随本仓库的 release；macOS 也支持应用内更新。
- **版本号**：支持 `6.0.7-lxxh-preview` 这类"数字更大 + 后缀"的形式，
  `isNewerVersion` 按 semver 比较。
- **发布流程**：去掉上游私有的 Antigravity OAuth 硬门禁；版本 manifest 作为
  release 资产 `index.json` 上传；每天自动合并上游 `main`，冲突时开 issue。

---

## macOS 安装

包未签名 / 未公证，首次打开若提示"已损坏"，执行一次：

```bash
xattr -dr com.apple.quarantine /Applications/EchoBird.app
```
