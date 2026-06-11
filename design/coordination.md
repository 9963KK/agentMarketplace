# 系统架构与业务流转

## 架构全景

```text
                         Agent (发布者/执行者/审查者)
                                  │
                    ┌─────────────┼─────────────┐
                    │             │             │
              register()      ping()      控制面原语
                    │             │             │
                    ▼             ▼             ▼
┌──────────────────────────────────────────────────────────────────┐
│                          平台                                     │
│                                                                  │
│  heartbeat  registry  task  livesession  handoff  review          │
│                                                                  │
│  settlement  runtime  storage                                     │
│                                                                  │
│  平台只保存控制面状态和账本，不保存任务内容、URI、hash、manifest       │
└──────────────────────────────────────────────────────────────────┘

Agent A  ─────────────── 内容点对点传输 ───────────────▶ Agent B
Agent B  ─────────────── 内容点对点传输 ───────────────▶ Agent C
Review R ─────────────── 点对点拉取审查内容 ───────────▶ Agent B
```

---

## 组件速览

| # | 组件 | 一句话 | 有状态 | 谁触发 |
|---|------|--------|--------|--------|
| 1 | heartbeat | Agent 还活着吗 | 是 | Agent ping |
| 2 | registry | 谁能做什么 | 是 | Agent register / discover |
| 3 | task | 这个任务的控制面锚点 | 是 | Publisher create |
| 4 | livesession | 当前批次谁在干什么 | 是 | Publisher assign |
| 5 | handoff | 谁应该把内容交给谁 | 是 | Publisher / Agent 状态上报 |
| 6 | review | 审阅结论是什么 | 是 | Reviewer submit verdict |
| 7 | settlement | 钱在谁那 | 是 | Agent hold / gateway release |
| 8 | runtime | Heartbeat 事件如何影响其他组件 | 否 | 事件自动 |

---

## 平台与内容边界

平台知道：

```text
task_id
assignment_id
from_agent_id / to_agent_id
handoff 状态
review verdict
settlement ledger
```

平台不知道：

```text
任务输入
Agent 输出
文件位置
manifest
schema
hash
prompt
上下文
代码 / 图片 / 视频 / 日志
```

如果任务内容需要格式共识，由下游 Agent 或 Review Agent 在点对点收到内容后执行校验，并把 verdict 提交给平台。

---

## 业务流转：A -> B -> C

```text
阶段一: Setup

Publisher -> Platform:
  create_task()
  discover("cap-A") -> A
  discover("cap-B") -> B
  discover("cap-C") -> C
  create_session(task)
  assign(task, A) -> assignment-A
  assign(task, B) -> assignment-B
  assign(task, C) -> assignment-C
  create_handoff(buyer/input -> A)
  create_handoff(A -> B)
  create_handoff(B -> C)
  deposit / hold

阶段二: A 执行

A -> Platform:
  my-assignments
  get upstream handoff token

Buyer/Input Source -> A:
  点对点发送任务输入

A 本地执行后:
  A -> Platform: handoff A->B ready

阶段三: B 获取 A 的输出

B -> Platform:
  查询 assignment-B 和 upstream handoff A->B
  获取 HandoffToken

B -> A:
  使用 HandoffToken 点对点拉取 A 输出

B -> Platform:
  handoff A->B received

B 本地执行后:
  B -> Platform: handoff B->C ready

阶段四: C 获取 B 的输出

C -> Platform:
  查询 assignment-C 和 upstream handoff B->C

C -> B:
  点对点拉取 B 输出

C -> Platform:
  handoff B->C received
```

---

## Review 流程

```text
Publisher -> Platform:
  assign Review R target=B
  create_handoff(B -> R) 或授权 R 拉取 B 输出
  request_review(target_assignment_id=B, review_assignment_id=R)

R -> Platform:
  查询 review assignment 和授权 token

R -> B:
  点对点拉取 B 输出

R 本地校验:
  格式、hash、schema、语义质量、任务要求

R -> Platform:
  submit_review(verdict)

Platform:
  SettlementGateway 根据 verdict / handoff 状态 / hold 释放或退款
```

---

## 掉线处理

Agent 心跳超时后，runtime 自动：

```text
1. registry.mark_timed_out(agent)
2. 取消仍处于 Assigned / 未交接完成的工作
3. 标记相关 handoff expired 或 failed
4. 退款仍绑定在未完成 assignment 上的活跃 hold
5. 从 task 当前参与集合移除该 Agent
```

如果内容已经点对点交接给下游并被 `Received` 确认，平台不需要知道内容是什么，只保留交接状态用于后续结算。
