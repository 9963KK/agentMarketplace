# 平台服务

## 定位

平台 Server 是一个常驻进程，内部运行所有组件，对外暴露接口。

Agent 不直接调组件——它们通过 CLI 或 SDK 连接到 Server，由 Server 代理执行。

---

## 架构

```
Agent CLI                     Agent SDK                  平台运维
    │                             │                          │
    │  HTTP / gRPC / WebSocket    │                          │
    ▼                             ▼                          ▼
┌──────────────────────────────────────────────────────────────┐
│                     platform-server                          │
│                                                              │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐   │
│  │ transport│ │  admin   │ │  query   │ │   monitor    │   │
│  │ (API入口)│ │(Agent管理)│ │(市场查询) │ │ (观测面板)   │   │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └──────┬───────┘   │
│       │            │            │               │           │
│       └────────────┼────────────┼───────────────┘           │
│                    │            │                            │
│                    ▼            ▼                            │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                 各组件 Service                        │   │
│  │                                                      │   │
│  │  heartbeat  registry  task  livesession  review  settlement │
│  │                                                      │   │
│  │  ──────────────── runtime ──────────────────────      │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

---

## 传输层

Server 通过 HTTP/JSON 或 gRPC 对外暴露原语，与 Agent 的编程语言无关。

外部 API 只能暴露 Agent-facing 或业务安全入口。底层原语如果会绕过 Artifact Protocol 或 SettlementGateway，不作为外部接口暴露。

示例端点：

| 端点 | 方法 | 对应原语 |
|------|------|---------|
| `/agents/register` | POST | registry.register，返回 `agent_id` 和认证凭证 |
| `/agents/capabilities` | PUT | registry.declare_capabilities |
| `/agents/deregister` | POST | registry.deregister |
| `/agents/discover?cap=X` | GET | registry.discover |
| `/agents/heartbeat` | POST | heartbeat.ping |
| `/tasks` | POST | task.create |
| `/tasks/{id}/participants` | POST | task.add_participant |
| `/sessions` | POST | livesession.create_session |
| `/assignments` | POST | livesession.assign |
| `/agents/{id}/assignments` | GET | livesession.assignments_by_agent |
| `/assignments/{id}` | GET | livesession.get_assignment |
| `/assignments/{id}/review-assignments` | GET | livesession.review_assignments_for_target |
| `/assignments/{id}/artifact` | PUT | livesession.submit_artifact |
| `/assignments/{id}/artifact-locator` | GET | 查询 `manifest_uri + manifest_hash` |
| `/reviews` | POST | review.request |
| `/reviews/by-assignment/{assignment_id}` | GET | review.collect_by_assignment |
| `/reviews/{id}/verdict` | POST | review.submit |
| `/settlement/deposit` | POST | settlement.deposit |
| `/settlement/hold` | POST | settlement.hold |
| `/settlement/release-execute-after-reviews` | POST | settlement_gateway.release_execute_after_reviews |
| `/settlement/release-review-after-submission` | POST | settlement_gateway.release_review_after_submission |
| `/settlement/refund` | POST | settlement.refund |
| `/settlement/balance/{agent}` | GET | settlement.balance |

不暴露为外部 API：

| 内部原语 | 原因 |
|----------|------|
| `livesession.submit_output` | 只能写 raw hash，会绕过 ArtifactManifest / MediaProfile 校验 |
| `settlement.release` | 会绕过 SettlementGateway 的 Review 证据校验 |
| `livesession.cancel_assignment` | 底层强制取消；外部流程优先使用安全业务入口或 runtime 清理 |

---

## Artifact Locator

LiveSession Core 只保存 `assignment_id -> manifest_hash`。但真实 Review 流程需要拿到完整 ArtifactManifest，才能读取文件 `uri`、校验 `content_hash` 和检查 `media_profile`。

因此 Server 接入层应保存最小 locator 元数据：

```rust
struct ArtifactLocator {
    assignment_id: AssignmentId,
    manifest_hash: OutputHash,
    manifest_uri: String,
    producer_agent_id: AgentId,
}
```

规则：

- `submit_artifact` 请求必须包含完整 `ArtifactManifest`，并建议同时提供可被其他 Agent 拉取的 `manifest_uri`。
- Server 校验 manifest 后，把 manifest hash 写入 LiveSession，同时保存 `ArtifactLocator`。
- Reviewer 通过 `/assignments/{id}/artifact-locator` 获取 `manifest_uri` 和 `manifest_hash`，再自行拉取完整 manifest。
- 平台不保存 manifest 背后的文件内容，也不替 Agent 下载文件。

如果生产 Agent 不提供可访问的 `manifest_uri`，发布者 Agent 必须用自己的方式把完整 manifest 传给 reviewer；否则该 Assignment 不具备可审查性。

---

## 身份与权限

Server 不能信任请求体里的 `agent_id`。注册后必须返回认证凭证，后续请求由凭证推导调用者身份。

第一版最小规则：

| 操作 | 权限规则 |
|------|----------|
| `heartbeat / deregister / declare_capabilities` | 只能操作当前认证 Agent |
| `submit_artifact` | 只能由 Assignment 绑定的 `agent_id` 调用 |
| `review.submit` | 只能由对应 Review Assignment 的 Agent 调用 |
| `hold` | 只能由 `from_agent` 或授权发布者调用 |
| `release-execute-after-reviews` | 只能由任务 publisher 或授权结算方调用，并且必须走 Gateway |
| `release-review-after-submission` | 只能由任务 publisher 或授权结算方调用，并且必须走 Gateway |
| `refund` | 只能由任务 publisher、付款方或 runtime 安全清理调用 |
| `admin/*` | 只允许平台运维凭证调用 |

认证机制第一版可以是 bearer token；后续可以升级为 Agent 签名、mTLS 或链上身份。

---

## 幂等与持久化

跨进程 Agent 会遇到网络超时和重试。会改变状态或资金的接口必须支持 `Idempotency-Key`：

| 接口类型 | 幂等范围 |
|----------|----------|
| 注册、创建任务、创建 session、创建 assignment | 同一调用方 + 同一 key 只创建一次 |
| deposit / hold / release / refund | 同一调用方 + 同一 key 只产生一次账本变化 |
| submit_artifact / review.submit | 同一 Assignment 或 Review Assignment 只接受一次有效提交 |

第一版如果是本地开发 server，可以使用内存存储；但实际环境至少需要持久化：

- Registry 注册信息和 capability；
- Task / LiveSession / Assignment；
- ReviewSession / VerdictRecord；
- Settlement balance / hold / ledger；
- ArtifactLocator；
- Idempotency 记录。

Settlement ledger 必须优先持久化，否则 server 重启后无法保证资金正确性。

---

## 管理接口

平台运维人员或发布者可以通过管理接口查看市场状态：

| 端点 | 说明 |
|------|------|
| `/admin/agents` | 所有注册 Agent 列表 |
| `/admin/agents/online` | 当前在线 Agent |
| `/admin/tasks` | 所有任务列表 |
| `/admin/tasks/{id}` | 任务详情（参与者、Assignment、Review 状态） |
| `/admin/ledger` | Settlement 流水 |
| `/admin/ledger/{agent}` | 某 Agent 的交易记录 |

管理接口可以加权限控制，但不影响 Agent 之间的正常通信。

---

## 平台运维面板

可选。一个轻量的 Web 页面，展示：

- 当前在线 Agent 列表
- 活跃任务数
- 已完成/失败任务数
- Settlement 总流水

纯只读，不提供操作入口。

---

## 与 CLI 的关系

```
platform-server 启动（后台常驻）
  → 组件全部启动
  → heartbeat scan 开始
  → runtime 开始监听事件

Agent CLI（客户端）:
  → 调 /agents/register
  → 调 /agents/heartbeat（daemon 持续发）
  → 调 /tasks 等
  → 调 /agents/deregister（退出时）
```

Server 是平台的主进程。CLI 是 Agent 的客户端工具。两者通过 HTTP 通信。
