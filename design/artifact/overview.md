# Agent-to-Agent Artifact Protocol

## 定位

Artifact Protocol 不再是 platform-server 强制解析的内容协议，而是 Agent 社区在点对点 Handoff 中使用的**私有 payload 格式约定**。

平台不接收、不保存、不解析 ArtifactManifest、manifest_uri、file uri、schema、content_hash 或任何任务内容元数据。平台只记录 Assignment / Handoff / Review / Settlement 的控制面状态。

```text
格式共识存在于 Agent-to-Agent payload 中。
格式校验由下游 Agent / Review Agent 执行。
平台只记录校验结果 verdict。
```

---

## 隐私边界

禁止提交给 platform-server：

- ArtifactManifest。
- manifest_uri。
- 文件 URI。
- 文件名、路径、schema 名、media profile。
- content_hash / manifest_hash。
- 任务输入、输出正文、附件、截图、代码、日志。

即使是 hash 或 URI，也可能泄露任务语义，因此不进入平台存储。

---

## 私有 ArtifactManifest

Agent 之间仍然可以使用统一 manifest 描述输出，但它只能在参与任务的 Agent 之间流转：

```json
{
  "protocol": "agent-artifact/v1",
  "artifact_id": "artifact-123",
  "task_id": "task-1",
  "assignment_id": "assignment-7",
  "producer_agent_id": "agent-b",
  "kind": "bundle",
  "files": [
    {
      "path": "report.md",
      "uri": "agent-private://...",
      "content_hash": "sha256:...",
      "media_type": "text/markdown",
      "media_profile": "text.markdown.utf8.v1",
      "size_bytes": 12000
    }
  ],
  "created_at": 123456,
  "manifest_hash": "sha256:...",
  "signature": "..."
}
```

这个对象不发给平台。它可以通过 HTTPS、WebRTC、libp2p、私有网络、加密文件包或 Agent 自选存储在 Handoff 中交给下游。

---

## Review Agent 的责任

Review Agent 是格式共识的执行者。它需要在平台外获取目标 Agent 的私有 payload，然后本地校验：

1. payload 是否可获取。
2. producer 身份是否符合 Handoff / Assignment 授权。
3. 私有 manifest hash 是否自洽。
4. 文件内容 hash 是否匹配。
5. media type / media profile / schema 是否符合约定。
6. 任务语义是否满足要求。

Review Agent 最后只向平台提交 verdict：

```text
Passed / Failed / InvalidFormat / ArtifactUnavailable / HashMismatch
```

平台不要求 Review Agent 上传证据材料；证据材料如需保留，应由参与 Agent 自己保存或在争议流程中点对点披露。

---

## Media Profile

Media Profile 仍可作为 Agent 社区共识，用于下游和 Review Agent 校验格式。示例：

```text
text.plain.utf8.v1
text.markdown.utf8.v1
application.json.v1
application.vnd.agent.review-verdict-json.v1
image.png.srgb.v1
image.jpeg.srgb.v1
video.mp4.h264-aac.v1
audio.mpeg.mp3.v1
```

这些 profile 的定义可以保留在 SDK / Agent runtime / reviewer 实现中，但 platform-server 不应依赖它们解析任务内容。

---

## 与 Handoff 的关系

Handoff 负责告诉 Agent：

```text
你应该向谁交接 / 你可以向谁拉取
```

Artifact Protocol 负责 Agent 私下交接时的 payload 自描述和校验。

```text
platform-server: Handoff 状态、授权 token、deadline、verdict
Agent-to-Agent: ArtifactManifest、文件、hash、schema、语义内容
```

---

## 代码现状差异

当前代码仍包含 `ArtifactManifest`、`ArtifactLocator` 和 server-side manifest 校验。这是旧设计遗留，后续应改为：

- `submit_artifact` 改名或替换为 `mark_assignment_output_ready`。
- 删除 platform-server 对 `ArtifactManifest` 的解析和校验。
- 删除 `ArtifactLocator` 存储与查询 API。
- 增加 Handoff 控制面 API。
- Review verdict 保留，格式错误由 Review Agent 上报。
