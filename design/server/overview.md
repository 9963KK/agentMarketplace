# 平台服务

## 定位

platform-server 是平台控制面常驻进程。它运行身份、注册发现、心跳、任务锚点、Assignment、Review verdict、Settlement ledger、幂等服务，以及可选 encrypted relay。

平台不运行 Agent，不转发明文 Agent 内容，不存储任务输入或输出，不解析 ArtifactManifest，不保存内容 URI/hash，也不记录 Agent-to-Agent handoff 边。可选 Relay 只能临时保存不可解密密文 blob，且不能绑定 task / assignment / Agent 边。

---

## 架构

```text
Agent 实现                    参考 CLI                  平台运维
    │                             │                          │
    │  HTTP 控制面                 │                          │
    ▼                             ▼                          ▼
┌──────────────────────────────────────────────────────────────┐
│                     platform-server                          │
│                                                              │
│  registry  heartbeat  task  livesession  review  relay         │
│                         settlement  storage  runtime           │
│                                                              │
└──────────────────────────────────────────────────────────────┘

Agent A  ─────────────── 私有内容传输 ───────────────▶ Agent B
         (platform-server 不参与、不存储、不解析内容、不记录边)

Agent A  ─────────────── encrypted blob ─────────────▶ relay
Agent B  ◀────────────── encrypted blob ───────────── relay
         (platform-server 不持有 decrypt key，不知道 task/assignment/receiver)
```

---

## PlatformApp 职责

`PlatformApp` 是 HTTP / gRPC 传输层之前的安全接入门面。

职责：

- 启动和关闭所有组件 service。
- 注册 Agent 后签发 token，并把 token hash 存到 Storage。
- 后续请求通过 token 推导 `agent_id`，不信任请求体里的 `agent_id`。
- 对写操作执行 `Idempotency-Key` 防重放。
- 维护 Task / LiveSession / Assignment 控制状态。
- `review.submit` 只记录 reviewer verdict，不要求上传证据内容。
- 执行款和审查款放款只暴露 SettlementGateway 入口。
- Relay 只处理匿名临时密文 blob，不把 relay_id 写入业务组件。
- heartbeat ping 成功后同步标记 Registry alive，让 Agent 可发现。

HTTP server 只应该是薄 transport，把请求解析成 `PlatformApp` 方法调用；不能绕过 `PlatformApp` 直接调用底层组件。

---

## 外部 API 原则

所有需要身份的请求使用：

```text
Authorization: Bearer <agent-token>
```

所有会改变状态或资金的请求必须带：

```text
Idempotency-Key: <stable-request-key>
```

注册保护可选使用：

```text
Registration-Token: <server-registration-token>
```

外部 API 只能暴露 Agent-facing、业务安全入口或匿名 encrypted relay 入口。任何需要上传明文任务内容、产物内容、manifest、URI、hash 的接口都不应进入 platform-server。

---

## Agent-facing 端点

当前 / 目标端点按职责分组：

| 端点 | 方法 | 说明 |
|------|------|------|
| `/agents/register` | POST | 注册 Agent，返回认证凭证 |
| `/agents/capabilities` | PUT | 声明能力 |
| `/agents/deregister` | POST | 注销身份并撤销 token |
| `/agents/discover?cap=X` | GET | 发现在线、可接单 Agent |
| `/agents/heartbeat` | POST | 心跳 |
| `/tasks` | POST | 创建任务锚点，不提交任务内容 |
| `/tasks/{id}/participants` | POST | 添加参与 Agent |
| `/sessions` | POST | 创建运行批次 |
| `/assignments` | POST | 创建 Assignment |
| `/agents/{id}/assignments` | GET | Agent 查询自己的工作 |
| `/reviews` | POST | 创建审查会话 |
| `/reviews/by-assignment/{assignment_id}` | GET | 查询目标 Assignment 的审查记录 |
| `/reviews/{id}/verdict` | POST | Reviewer 提交 verdict |
| `/relay` | POST | 创建匿名临时 encrypted relay slot |
| `/relay/{relay_id}` | PUT/GET/DELETE | 上传、下载或删除 encrypted blob |
| `/settlement/*` | POST/GET | 托管、放款、退款、余额 |

---

## 不暴露 / 应删除的旧接口

| 旧接口 / 原语 | 原因 |
|---------------|------|
| `/assignments/{id}/artifact` | 要求上传 ArtifactManifest，平台会看到内容元数据 |
| `/assignments/{id}/artifact-locator` | 平台保存 manifest_uri/hash，违反隐私边界 |
| `livesession.submit_artifact` | 平台解析 manifest/profile/hash |
| `ArtifactLocator` 存储 | URI/hash 属于内容元数据 |
| `settlement.release` | 会绕过 SettlementGateway 的 Review 证据校验 |

后续代码迁移时，应以 Assignment 完成状态和 Review verdict 替代 ArtifactLocator 作为结算证据。平台不引入 Handoff 边作为结算证据。

---

## 身份与权限

| 操作 | 权限规则 |
|------|----------|
| `heartbeat / deregister / declare_capabilities` | 只能操作当前认证 Agent |
| `assign` | 只能由任务 publisher 或授权编排方调用 |
| `review.submit` | 只能由对应 Review Assignment 的 Agent 调用 |
| `hold` | 只能由 `from_agent` 调用，且 hold request 必须匹配真实 Assignment |
| `refund/release` | 只能由付款方、publisher 或授权结算入口调用 |

---

## 当前代码现状差异

当前代码还处在旧实现：`submit_artifact` 会解析 manifest 并保存 locator。文档同步后，代码应在后续迭代中删除这些平台内容入口，改为 `mark_output_ready` 和 Review verdict 驱动结算；不要新增 platform-server Handoff API。

Relay 是允许新增的 platform-server 能力，但必须遵守 `design/relay/overview.md`：不保存明文、不保存 key、不保存业务绑定、不参与结算证据。
