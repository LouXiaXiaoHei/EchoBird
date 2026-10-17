# EchoBird New Tool Integration Playbook

Last validated: 2026-10-17, EchoBird 5.9.8.

Use this guide when adding or extending a tool across the full product path:

```
official product discovery
  -> catalog and icon
  -> detection / install / launch
  -> Mother Agent install guidance
  -> API model read / apply / restore
  -> account login and native switching
  -> account-backed model catalog
  -> reverse proxy and protocol conversion
  -> Smart Access routing
  -> UI, translations, tests, packaging
```

The goal is one shared lifecycle with only the vendor differences that are
actually required. Do not build a second account, model, installer, or routing
framework for a new provider.

## 1. Before writing code

Record a short scope table for Windows, macOS, and Linux. For each platform,
state whether each capability is:

- vendor-supported and implemented;
- implemented but not live-tested;
- not implemented;
- unavailable from the vendor.

Cover detection, install, launch, account login, native account switching,
quota, API model configuration, account proxy, and Smart Access.

Read the current official documentation and inspect the shipped client where
needed. Confirm all of the following before committing to an integration:

- the official installer and release channels;
- file paths and credential locations on each OS;
- whether CLI and desktop share configuration or credentials;
- whether the vendor exposes model lists, quota, or plan data;
- whether data sources or analysis features require separate paid services;
- open-source licenses and required notices;
- whether another client or running process owns the same files.

Feasibility is part of the product decision. A usable installer is not enough
if the core workflow depends on an unwanted paid-data path or a vendor-only
account the user cannot use.

### Non-negotiable behavior

- Entering a page or selecting a saved account may read local state only when
  the client is installed.
- Passive navigation must not start login, query quota, fetch a remote model
  catalog, change a client config, or open a blocking dialog.
- Explicit actions are login, refresh, apply, launch, restore, and delete.
- Unknown data remains unknown. Do not invent `Free`, zero quota, or a model ID.
- Preserve user fields, other providers, sessions, watchlists, themes, and
  credentials that are outside the requested change.
- Fixture success, a Windows build, a live API request, and a native GUI test
  are separate evidence. Report them separately.

## 2. Tool definition and install catalog

The main tool definition lives in:

- `tools/<id>/paths.json`
- `tools/<id>/config.json` when the normal config mapper is sufficient
- `public/icons/tools/<id>.*`
- `docs/icons/tools/<id>.*` when the public site needs the same icon

`paths.json` should describe the product precisely:

- `name`, localized `names`, category, website, and docs;
- `apiProtocol` and `noModelConfig`;
- `command`, `startCommand`, environment override, working-directory rules;
- `configDir`, `configFile`, and alternate files;
- platform paths for `win32`, `darwin`, and `linux`;
- install hints for Windows uninstall entries, MSIX, macOS app bundles, or Linux
  desktop entries;
- process names or launch URI when the executable name is not enough.

The external install guidance is:

- `docs/api/tools/install/<id>.json`
- add the same ID to `docs/api/tools/install/index.json`

The same reference is compiled into the app:

- add `<id>` to `BUNDLED.install_refs` in `src-tauri/src/lib.rs`;
- add `<id>` to `INSTALLABLE_TOOL_IDS` in
  `src-tauri/src/services/bundled_assets.rs`.

Do not update one copy and forget the other. The install index, compiled
references, and Mother Agent prompt are expected to stay in lockstep. The
existing consistency test
`install_index_compiled_references_and_mother_prompt_match` is the guardrail.

Write install guidance as an executable contract:

- verify OS, architecture, and whether the product is already installed;
- use the official release feed when current asset names are versioned;
- reject the wrong CLI, desktop, extension, or companion package;
- preserve existing user data during install and update;
- verify the resolved executable before claiming success;
- distinguish a running WebUI, Python checkout, or source tree from the official
  desktop client.

For Mother Agent, the install JSON should name prerequisites, download source,
verification step, and the boundary between "installed" and "started".

## 3. Detection, installation, and launch

Reuse:

- `src-tauri/src/services/tool_manager.rs`
- `src-tauri/src/services/msix.rs`
- `src-tauri/src/services/process_manager.rs`
- `src-tauri/src/utils/process.rs`

Detection must use the shared platform snapshot path. Do not add a provider-
private registry walk or an unverified default-path guess. The tool is installed
only when an executable, bundle, Store package, or authoritative client state is
present. A stale config directory alone is not proof, especially after uninstall.

Installation should be idempotent. If a tool supports a custom installation
directory, honor registry hints and the user override; do not require the default
location. For MSIX, resolve the current AUMID and version instead of pinning a
single Store identity when the vendor publishes variants.

Launch through the common process manager. Use the GUI path, start command,
launch URI, or working directory appropriate to the product. Keep interactive
terminal launches separate from background command helpers.

Do not:

- start or stop a client merely from account/session inspection;
- claim a launch succeeded because a process was spawned briefly;
- kill unrelated processes with a broad image-name match;
- write into a running client's config without checking its process and
  ownership;
- use a hidden background command when the user explicitly needs an interactive
  terminal.

## 4. API model configuration

Set `noModelConfig: false` and the supported `apiProtocol` values only when the
tool actually reads and applies API model configuration.

For a standard JSON/TOML/env layout, start with `tools/<id>/config.json` and the
generic mapper. Add `src-tauri/src/services/tool_config_manager/<id>.rs` only
when the vendor uses a nonstandard store, multiple files, Electron storage,
SQLite, a proprietary format, or difficult conflict rules.

The provider dispatch must implement three explicit operations:

- read the current model;
- apply the selected model;
- restore the official/default mode.

Add all three to `tool_config_manager.rs` and any path resolution needed by
`tool_manager.rs`. If a model change requires a client restart, add the tool to
the managed desktop restart rules instead of asking the frontend to duplicate the
process logic.

For every write:

- preserve unknown fields and unrelated providers;
- normalize the vendor's endpoint shape, not just the visible base URL;
- use the same canonical OpenAI/Anthropic URL rules as the rest of EchoBird;
- use an atomic transaction for paired files;
- retain a rollback path for partial writes;
- never fabricate a model ID or key;
- avoid a passive read that mutates the file or a database lock.

Manage vendor-specific `.env` semantics carefully. Empty values can mean
"automatic", "disabled", or "no fallback" depending on the client. A visual or
auxiliary model must not become the fallback for the primary model unless that is
the confirmed vendor contract.

Anthropic and OpenAI model controls are not global defaults merely because they
share a slider. Send the protocol-specific field. The user's current convention
is five reasoning levels plus an omitted/default selection for every API model;
default means omit the parameter and let the provider decide. Do not add a
DeepSeek-only or model-name-only exception.

## 5. Account login and native switching

Reuse:

- `src/pages/AppManager/useManagedAccounts.ts`
- `src/pages/AppManager/AccountSectionPrimitives.tsx`
- `src/pages/AppManager/<provider>/AccountSection` or the shared client-account
  section
- `src/api/accountProxy.ts` and the shared account model fields

The provider hook supplies only the real differences:

- OAuth/device-code/cookie/browser callback;
- region or edition;
- stable account identifier;
- native credential location;
- apply/restore adapter;
- plan, quota, and balance presentation fields.

Do not copy the 60-second timer, cancellation, late-response guard, list loading,
selection, refresh, delete, or navigation behavior into a provider-specific hook.

Required lifecycle:

1. Add an account and save it in EchoBird.
2. Explicitly apply the saved account to the native client.
3. Keep account selection and API-model selection mutually exclusive.
4. Cancel login on leaving the tool or timeout; ignore late results.
5. Keep refresh/delete separate from row selection.
6. Preserve cached rows after passive read failures.
7. Show a blocking error only for explicit failures, not passive inspection.
8. Never expose raw tokens to the frontend, logs, fixtures, or release notes.

Regions and editions are not cosmetic. Key OAuth state, caches, credentials,
account IDs, and native writes by both provider and region/edition. Switching a
region while a login is in flight must not let the old response attach to the new
selection.

Quota parsing must distinguish:

- a valid empty success payload;
- an unknown or unavailable quota;
- a zero value returned explicitly;
- an authentication or network failure.

An empty success is not an error. A missing value is `—`, not zero. A free account
may legitimately have no quota rows. Do not infer a plan from an empty response.

## 6. Account-backed models and reverse proxy

The account model catalog starts in
`src-tauri/src/services/account_model_sources.rs`. Add the provider to the shared
source enum and fetch the vendor's dynamic model list only through an explicit
catalog action or a previously cached catalog.

Add the protocol adapter under
`src-tauri/src/services/account_proxy/<provider>.rs` and dispatch it from
`account_proxy/providers.rs`. Keep the saved account source and model ID in the
model record; never store a copied raw credential in the model config.

The adapter should handle the actual vendor contract:

- authentication headers refreshed per request;
- one shared client with explicit redirect and timeout policy;
- Chat, Responses, or Anthropic Messages selection according to the catalog;
- SSE streaming;
- tool definitions and tool-result continuation;
- reasoning/thinking fields;
- cancellation and upstream error status;
- model directory cache keys that include provider, region, and account.

Do not construct a new HTTP client for every request when the connection pool can
be shared. Do not put a single account's credentials in global default headers.

Model IDs and keyless/free labels come from the catalog or account provider
metadata, not from a hardcoded list of IDs. If a provider switches from one model
to another, the label must remain intact unless the provider metadata actually
changes.

## 7. Smart Access

Smart Access receives a `Candidate` containing at least the model ID, endpoint,
credential source, account source, and a fingerprint that changes when identity or
configuration changes.

Route selection must:

- prefer the configured order while respecting health/cooldowns;
- distinguish model-scope failures from account-scope failures;
- invalidate health and caches when an account is re-logged, forgotten, or
  switched;
- retry only when the request body and protocol can be safely repeated;
- preserve streaming and tool-call semantics;
- retain Responses continuation bindings within the documented limit;
- write route state outside the shared routing mutex when the codebase supports
  ordered snapshots.

Test with a controlled local HTTP server before testing a real account:

- text and streaming;
- forced tool call;
- tool result continuation;
- missing/invalid credentials;
- 401/403/429/5xx and `Retry-After`;
- a late response after cancellation;
- a changed account token invalidating an old route;
- no passive request during page navigation.

Never use a real paid account for a catalog or routing test unless the user
explicitly authorizes live inference and understands the cost.

## 8. UI and localization

Use the existing account section, model picker, App Desktop right panel, model
center, and Smart Access views. A new provider should normally add:

- an account section entry and translated plan/quota labels;
- provider icon assets in every path that preloads them;
- a catalog entry and category;
- model protocol options and account fields;
- the same account source in Account Center and Smart Access;
- all four locales: `zh-Hans`, `zh-Hant`, `en`, and `ja`.

Do not add a custom hover tooltip, cursor override, or explanatory paragraph when
an established control already exists. Keep click/keyboard events isolated from
selection and pending actions.

## 9. Verification matrix

Run the focused tests first, then the full repository gates:

```powershell
npm run typecheck
npm run format:check
npm run lint
npm test

cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

For an integration, add evidence for:

| Layer          | Minimum evidence                                                            |
| -------------- | --------------------------------------------------------------------------- |
| Catalog        | ID exists in bundled and remote indexes; Mother prompt contains it          |
| Path/detection | default, custom, Store/MSIX, uninstall, stale config                        |
| Install        | official asset selection, exit/error handling, user-data preservation       |
| Launch         | executable/bundle/URI resolution, working directory, process ownership      |
| API model      | read/apply/restore, protocol fields, unknown-field preservation, rollback   |
| Account        | login success, cancel, timeout, late response, region isolation             |
| Quota          | empty success, unknown, zero, explicit failure, cached-row preservation     |
| Proxy          | directory, text, stream, tools, tool results, errors, cancellation          |
| Smart Access   | candidate selection, cooldown, fallback, stale account invalidation         |
| Navigation     | no login, quota, catalog, config write, or blocking dialog on passive entry |
| Packaging      | resources present, version correct, no credentials/test artifacts           |

For every "must not happen" assertion, include a positive control that proves the
test can observe the action when it is intentionally triggered.

Record separately:

- unit/fixture results;
- Windows native build and UI result;
- macOS native result;
- Linux native result;
- real account login and quota result;
- live provider inference and cost boundary;
- release CI result.

## 10. Packaging and handoff

Before handoff, check:

- `tools/<id>` is included by `bundle.resources` and resolves in both source and
  installed layouts;
- icon files are present in `public` and installed resources where required;
- no absolute checkout paths, temporary probes, secrets, or account details were
  committed;
- the install catalog is in both bundled and remote indexes;
- the release version and notes do not claim unverified platform behavior;
- the current client/PID/Vite session and DSA or other local-only archives are
  recorded in `internal-docs/development-handoff.md`, not mixed into release
  claims.

Use this handoff block in a local note:

```
Goal:
Approved behavior:
Explicitly excluded behavior:
Files/modules touched:
Platforms implemented:
Platforms live-tested:
Fixture tests:
Live account/inference tests:
Known limitations:
Git state / archive branches:
Next action:
```

## 11. Pitfalls seen in real integrations

| Symptom                                              | Root cause to check first                                                                   |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Tool appears in the catalog but shows as uninstalled | Wrong platform path, stale install hint, or mismatch between bundled and remote catalog     |
| Installed app cannot launch                          | Missing working directory, wrong `startCommand`, MSIX AUMID drift, or unverified executable |
| Mother Agent installs a different CLI/desktop        | Install reference names the wrong product or omits identity guard                           |
| Model appears to save but the client ignores it      | Wrote a sibling file while the client reads another store                                   |
| Switching accounts loses models or watchlists        | Adapter rewrote the whole config instead of preserving unknown fields                       |
| Free account shows a quota error                     | Valid empty success was treated as malformed; inspect the actual shape                      |
| Keyless label disappears                             | Label was tied to one hardcoded model ID instead of provider metadata                       |
| Old login response selects the wrong account         | Missing login/selection revision guard                                                      |
| Two regions share credentials or model cache         | Cache/credential key lacks region/edition                                                   |
| Proxy works for text but breaks tools                | Endpoint/protocol conversion loses tool definitions or tool-result messages                 |
| Fallback uses the wrong model                        | Provider's empty-list/automatic-fallback semantics were not represented                     |
| Page switching causes network or config writes       | Passive entry was allowed to refresh or apply state                                         |
| Tests pass but packaged app differs                  | `_up_/tools` mirror stale; verify compiled/source resource discovery                        |
| Account deletion leaves a model route alive          | Account epoch/fingerprint not invalidated in route memory                                   |

### Case notes from the 5.9.7–5.9.8 work

- Kimi required one login entry covering `kimi.com` and `kimi.ai`, shared CLI and
  desktop config, five real plan labels, and a valid empty quota response. It also
  showed why region/account keys and empty-success handling need dedicated tests.
- The finance-app pilot showed that provider data-source and paid-upgrade
  requirements must be evaluated before shipping. It was preserved only in a
  local archive branch and removed from main; the reusable lesson is feasibility
  first, not a claim of support.

## 12. Definition of done

An integration is done only when:

- the tool is represented in both catalogs and the compiled Mother prompt;
- the supported platforms are explicit;
- detection/install/launch work through shared services;
- API configuration or account login exists only if the user asked for it;
- account-backed models use dynamic catalogs and no copied secrets;
- Smart Access works with the account source and survives cancellation/errors;
- passive navigation has no remote or write side effects;
- the four language surfaces and icons are complete;
- focused and full gates pass;
- live verification limits are stated without upgrading fixture evidence;
- the local handoff note records any excluded or archived scope.
