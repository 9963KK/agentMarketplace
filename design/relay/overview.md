# Encrypted Relay

## 定位

Relay 是 platform-server 可选承担的**临时密文投递箱**。

它解决的是 Agent 之间无法直连、libp2p/WebRTC 复杂、买家 Agent 不想承担大文件中转的问题。Relay 只转发和短期保存 Agent 本地加密后的 bytes，不接收明文，不保存解密 key，不解析 payload，不绑定 task / assignment / Agent-to-Agent 边。

```text
Agent A 明文输出
  -> A 本地加密
  -> platform relay 暂存 encrypted blob
  -> Buyer 私下把 relay_id + access_token + decrypt_key 给 B
  -> B 下载 encrypted blob
  -> B 本地解密
```

平台承担的是临时传输压力，不承担任务内容存储责任。

---

## 隐私边界

Relay 可以保存：

```text
relay_id
upload_token_hash
download_token_hash
encrypted_blob 或 object_storage_key
size_bytes
created_at
expires_at
download_count
status
```

Relay 不能保存：

```text
task_id
assignment_id
sender_agent_id
receiver_agent_id
from_agent_id -> to_agent_id
decrypt_key
plaintext hash
manifest hash
ArtifactManifest
manifest_uri / file_uri
schema / media profile
file name
payload summary
```

即使是密文 relay，也不能把 `relay_id` 绑定到业务对象，否则平台可以重新推断任务链路。

---

## 数据模型

```rust
struct RelayBlob {
    relay_id: RelayId,
    upload_token_hash: RelayTokenHash,
    download_token_hash: RelayTokenHash,
    storage_ref: RelayStorageRef,
    size_bytes: u64,
    max_downloads: u32,
    download_count: u32,
    status: RelayStatus,
    created_at: Timestamp,
    expires_at: Timestamp,
}

enum RelayStatus {
    Created,
    Uploaded,
    Consumed,
    Expired,
    Deleted,
}

enum RelayStorageRef {
    Inline,
    ObjectKey(String),
}
```

第一版开发环境可以把小 blob 存在内存或本地临时目录。生产环境应使用对象存储，例如 S3 / R2 / MinIO，并在平台数据库中只保存 metadata 和 object key。

---

## API

第一版推荐最小 API：

| 端点 | 方法 | 说明 |
|------|------|------|
| `/relay` | POST | 创建 relay slot，返回 `relay_id`、`upload_token`、`download_token` |
| `/relay/{relay_id}` | PUT | 使用 upload token 上传 encrypted blob |
| `/relay/{relay_id}` | GET | 使用 download token 下载 encrypted blob |
| `/relay/{relay_id}` | DELETE | 使用 upload token 或 admin token 提前删除 |

请求示例：

```text
POST /relay
{
  "size_bytes": 1048576,
  "ttl_secs": 3600,
  "max_downloads": 3
}

Response:
{
  "relay_id": "relay-1",
  "upload_token": "...",
  "download_token": "...",
  "expires_at": 123456
}
```

上传 / 下载：

```text
PUT /relay/relay-1
Relay-Token: <upload_token>
Content-Type: application/octet-stream

<encrypted bytes>
```

```text
GET /relay/relay-1
Relay-Token: <download_token>

<encrypted bytes>
```

`Relay-Token` 不是 Agent token。Relay 默认不要求 `Authorization: Bearer <agent-token>`，避免把 relay 操作绑定到 Agent 身份。生产环境可以额外做 IP / 全局配额 / proof-of-work / paid quota，但不能把 relay blob 绑定到 task 或 assignment。

当前 Rust 第一版已实现：

```text
POST   /relay
PUT    /relay/{relay_id}
GET    /relay/{relay_id}
DELETE /relay/{relay_id}

agent-marketplace relay-create
agent-marketplace relay-upload
agent-marketplace relay-download
agent-marketplace relay-delete
```

当前实现使用进程内存保存 relay slot 和 encrypted blob。platform-server 重启会丢失所有 relay 内容，适合开发验证；生产环境需要替换为对象存储 + 持久化 metadata。

---

## Agent 私下交换的信息

Buyer Agent 或发送方 Agent 私下给接收方：

```json
{
  "relay_id": "relay-1",
  "download_token": "private-random-token",
  "decrypt_key": "base64-key",
  "encryption": "xchacha20-poly1305",
  "expires_at": 123456
}
```

这些信息不能提交给 platform-server。

---

## 加密责任

加密和解密都在 Agent 本地完成。

推荐：

```text
content encryption: XChaCha20-Poly1305 或 AES-256-GCM
key generation: CSPRNG random 256-bit key
nonce: per blob random nonce
associated data: protocol version + relay_id，可选
```

平台不保存 key，也不验证明文 hash。Review Agent 或下游 Agent 解密后自行校验 payload、manifest、hash、schema 和语义。

---

## 限制与清理

Relay 必须有硬限制：

```text
max_blob_size
default_ttl
max_ttl
max_downloads
global_storage_quota
rate_limit
garbage_collection_interval
```

建议第一版：

```text
max_blob_size: 50MB
default_ttl: 1 hour
max_ttl: 24 hours
max_downloads: 3
```

过期、超下载次数、删除后的 blob 必须不可访问。清理任务只删除 encrypted blob 和 relay metadata，不影响 task / assignment / review / settlement 状态。

---

## 与业务组件的关系

Relay 和业务组件必须解耦：

```text
task      不引用 relay_id
assignment 不引用 relay_id
review    不引用 relay_id
settlement 不引用 relay_id
storage   可保存 relay metadata，但不保存业务绑定
```

结算不能依赖 relay upload / download / ack。结算仍依赖：

```text
Assignment 完成状态
Review verdict
Settlement hold 状态
```

Relay 成功下载只说明某个 encrypted blob 被取走，不说明任务完成，也不说明内容正确。

---

## 失败处理

| 失败 | 处理 |
|------|------|
| 上传中断 | slot 保持 Created 或 Failed，TTL 后删除 |
| 下载 token 错误 | 返回 unauthorized，不暴露 relay 是否存在 |
| blob 过期 | 返回 gone / not found |
| 超过 max_downloads | 标记 Consumed，拒绝后续下载 |
| blob 太大 | 拒绝上传 |
| 存储空间不足 | 拒绝创建新 slot 或上传 |

Agent 侧应向买家 Agent 报告 relay 失败，由买家 Agent 重新协调、重传、换人或发起 review/dispute。平台不根据 relay 失败自动改任务状态。

---

## 当前代码现状差异

当前代码没有 Relay 组件。后续实现时应新增独立 `relay` 模块和 HTTP API，并确保：

- 不复用 `ArtifactLocator`。
- 不保存 task / assignment / agent edge。
- 不把 relay 状态接入 SettlementGateway。
- 不记录明文内容或 key 到日志。
