# 版本改动记录

本仓库是 [edison7009/EchoBird](https://github.com/edison7009/EchoBird) 的个人自用
fork。与上游的整体差异见 [FORK.md](FORK.md)；这里按版本记录**每个发布改了什么**。

版本号形如 `x.y.z-lxxh-preview.N`，`N` 省略即 `.0`（semver 里
`-preview` < `-preview.1` < `-preview.2`）。上游功能请对照上游的 release notes。

---

## 6.0.6-lxxh-preview.4 — 2026-10-27

**修复：智能接入在慢请求上"自伤"——一次慢响应变成几分钟的 503**

现象：`503 All Auto Router models are cooling down`，之后无论怎么重试都是
503，过一会儿自己又好了。日志里的真实链条：

```
08:22:40  m-498cac timed out before response headers
08:22:55  m-1e235a timed out before response headers   ← 比上一条晚 15 秒整
08:22:55  all candidates failed: [transient, transient]
08:23:13  all candidates failed: []                    ← 两个都在冷却，零次尝试
```

三处根因，逐一修掉（`src-tauri/src/services/smart_router/server.rs`）：

1. **超时死线不随请求体增长**（`RoutePolicy::for_request` 新增）
   原来 `header_timeout = 30s` 固定；200+ 条消息、数百 KB 的上下文在上游
   prefill 阶段本来就常超过 30 秒，于是"慢"被判成"挂"。现在按序列化后的请求体
   放大：每 KiB +250ms，30s 起、120s 封顶（`HEADER_TIMEOUT_PER_KB` /
   `MAX_HEADER_TIMEOUT`）。
2. **总预算不够一次完整的 failover**（`MAX_TIME_BUDGET`）
   原来 `time_budget = 45s`，第一个候选吃掉 30s 后，第二个只剩 15 秒——注定
   超时，日志里两条超时正好差 15 秒。现在预算至少是 `header_timeout × 候选数`，
   10 分钟封顶。
3. **流式请求的首包死线从发请求时开始算**
   流式分支此前用 `header_timeout.min(first_content_timeout)`（30s）压住等待
   响应头，且首包死线在 `send_candidate` 之前就开始计时。现在等待响应头只受
   放大后的 `header_timeout` 约束，首包死线改在**收到响应头之后**才开始计
   `first_content_timeout`。

另外两处行为调整：

- **超时不再逐次翻倍冷却**（`mark_slow_attempt`）：超时记为慢请求，固定 30 秒
  冷却、不累加 `consecutiveFailures`，不会再从 30s 阶梯爬到 5 分钟。
- **全部候选冷却时不再直接 503**（`soonest_recovering` +
  `try_begin_attempt_despite_cooldown`）：挑最早解冻的那个候选试一次，只有它也
  失败才返回 503。绑定续写（`previous_response_id`）仍只走原候选。

验证：`cargo test --lib` 1900 通过（新增 3 条：死线按体量放大且预算覆盖每个
候选、慢请求不升级冷却、全冷却时回落到最早解冻的候选）、`cargo fmt --check`
干净、`cargo clippy` 无新增告警。修复前后 `.2/.3` 都在现场复现过，日志见上。

---

## 6.0.6-lxxh-preview.3 — 2026-10-25

**新增：智能接入支持 Anthropic 协议上游**

外部客户端（Claude Code 等）经智能接入发 `/v1/messages` 时，此前一律先转成
OpenAI 再转回来，Anthropic 协议专属能力必然丢失：`anthropic-beta`
（1M 上下文靠它开）被丢弃、`cache_control` 缓存标记被抹掉、模型 id 被改写。
现在按候选逐个选协议：

- 模型配了 `anthropicUrl`（且不是账号型候选）→ 请求体与 `anthropic-beta` /
  `anthropic-version` / UA 等请求头原样转发到上游 `/v1/messages`，返回体（JSON 与
  SSE）也原样回给客户端，不再经过 OpenAI→Anthropic 适配器；
- 其余候选仍走原来的 OpenAI 管线，行为不变。

候选顺序、failover、冷却记分、上下文超限直接 400、`X-EchoBird-Routed-Via`
诊断头都保留；透传流额外加空闲超时，避免上游静默挂死。
（`src-tauri/src/services/smart_router/server.rs`、新增
`src-tauri/src/services/smart_router/anthropic_native_tests.rs`）

**修复：窗口信息不再缺位**

- **`/v1/models`**：之前只回 `{id, owned_by}`，靠自动发现模型的客户端拿不到窗口，
  只能按自己最小的档位走。现在补上 `context_length` / `context_window` /
  `max_context_length` / `max_output_tokens` / `top_provider.*`，取值是「所有候选
  里的最小窗口」（请求可能 failover 到更小的那个，广播最大值会撑爆后备候选）。
- **OpenCode**：`echobird` provider 缺 `limit.context` / `limit.output`，1M 模型
  表现得像小模型。现在按真实窗口写入（output 取窗口 1/4，clamp 在 8K–64K 且不超过
  窗口一半）。
- **Codex**：`model_context_window` / `model_auto_compact_token_limit` 改用共享的
  窗口来源，与路由器、OpenCode 同源。kimi（256K）、claude（200K）、glm-5.1
  （200K）、gpt-5.1-codex（400K）等此前一律按 1M 写，Codex 在真实窗口一半左右就
  开始撞上下文；未知模型仍是 1M 默认。

窗口统一收进新的 `src-tauri/src/services/model_context.rs`：厂商目录（按 base_url
选对目录、按 slug 取对应条目）→ 家族兜底 → 1M 默认。

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
