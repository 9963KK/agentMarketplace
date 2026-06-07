# Review

审阅公证。只记录，不裁决。Review 审查的是 Chain 里登记的 `artifact_id` / `output_hash`，不保存输出正文。

## 原语

- `request(task_id, node_id, artifact_id, output_hash, criteria)` — 发起审阅
- `submit(review_id, agent_id, verdict)` — 提交裁决
- `collect(review_id)` — 查询全部裁决

## 平台规则

- 裁决只追加，不可覆盖
- 不存原文，存 artifact 引用和哈希防篡改
- 审阅 Agent 需要从 HolderCommitment 指向的 Agent 拉取内容并校验 hash

## 待细化

- 防篡改机制
- 与 settlement 的 release 校验联动
- 审阅 Agent 拉取 artifact 失败时的 verdict 表达
