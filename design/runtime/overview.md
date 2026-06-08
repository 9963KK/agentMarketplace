# Runtime

## 定位

Runtime 是平台原子组件的**接线层**。它不存业务状态，不编排任务链路，只把组件事件转换成固定的安全清理动作。

Runtime 的存在理由：有些反应不能依赖 Agent 主动调用。例如 Agent 掉线后，平台必须让它不可发现，并释放它名下仍然活跃的托管资金。

---

## 职责

| Runtime 负责 | Runtime 不负责 |
|---------------|----------------|
| 启动各组件 service | 任务链路编排 |
| 连接 Heartbeat 事件 | 选择具体 Agent |
| 执行掉线后的安全清理 | 判断审阅是否通过 |
| 统一 shutdown | 决定重做还是换人 |
| 记录运行时日志 | 定价、分账、充值 |

Runtime 只做平台红线的延伸：活跃检测后的清理动作。其他任务流程仍由发布者 Agent 自己决定。

---

## 事件协调

### Agent 超时

```text
Heartbeat: AgentTimedOut { agent_id }
  -> Registry.mark_timed_out(agent_id)
  -> LiveSession.assignments_by_agent(agent_id)
  -> filter assignment.status == Assigned
  -> LiveSession.cancel_assignment(assignment_id)
  -> Settlement.active_holds_for_agent(agent_id)
  -> filter hold.agent_id == agent_id
  -> filter hold.assignment_id was cancelled above
  -> Settlement.refund(hold_id)
  -> Task.active_tasks_by_agent(agent_id)
  -> Task.remove_participant(task_id, agent_id)
```

语义：

- Registry 只标记不可发现，不注销 Agent。
- LiveSession 只取消该 Agent 名下 `Assigned` 状态的 Assignment；已 `Submitted` 的输出保留，继续交给 review / settlement 流程处理。
- Settlement 只退款 `hold.agent_id == agent_id` 且对应 Assignment 已被本次 timeout 清理取消的 Active hold；Agent 作为付款方的 hold 不会因为付款方掉线被自动退款。
- Task 只从当前参与者集合移除该 Agent，不删除历史参与记录。
- Runtime 不判断这个 Agent 是 executor 还是 reviewer。

### Agent 恢复

```text
Heartbeat: AgentRecovered { agent_id }
  -> Registry.mark_alive(agent_id)
```

恢复只影响可发现性。Runtime 不自动把 Agent 加回任务，也不恢复已退款的 hold。

---

## 不做固定编排

Runtime 不提供这些操作：

```text
engage_executor
engage_reviewer
complete_task
cancel_task
retry_executor
replace_executor
replace_reviewer
```

原因：这些操作会把平台变成任务编排器，让平台理解执行者、审查者、阶段、重做、换人和放款时机。第一版坚持：这些决策由发布者 Agent 自己做。

发布者 Agent 应该直接按需调用原子组件：

```text
Task.create()
Registry.discover()
Task.add_participant()
LiveSession.create_session()
LiveSession.assign()
Settlement.hold(..., assignment_id, ...)
Review.request(..., target_assignment_id, review_assignment_ids, ...)
Review.collect_by_assignment()
Settlement.release(..., evidence)
Task.complete()
```

Runtime 不替它封装成业务流程。

---

## 不维护任务阶段

Runtime 不维护：

```text
Setup -> Execution -> Settlement -> Closed
```

阶段属于发布者 Agent 的任务策略。平台只保存 Task 自身状态：

```text
Active / Completed / Cancelled
```

`TaskStatus` 只表达任务元数据状态，不表达链路执行阶段。

---

## 与各组件关系

```text
Heartbeat -> Runtime -> Registry
                    -> LiveSession
                    -> Settlement
                    -> Task
```

Runtime 只消费 Heartbeat 事件。它不消费 Review 事件，不读取 Review verdict，不主动 release executor。

Settlement 的 `release()` 仍由发布者 Agent 调用，并由发布者提供 `ReleaseEvidence`。

---

## 错误处理

Runtime 对单个清理动作失败不应阻塞其他清理动作：

```text
refund(hold-1) failed
  -> 记录错误
  -> 继续 refund(hold-2)
  -> 继续 Task.remove_participant(...)
```

第一版可以只做内存内日志或返回错误列表。后续如果需要可靠投递，再增加 outbox / retry 机制。

---

## 暂不做

- 任务流程 API
- 阶段状态机
- 自动选择 Agent
- 自动判定 Review 是否通过
- 自动 release executor
- 自动 deposit
- 可靠事件 outbox
- 分布式协调

第一版 Runtime 只做无状态接线和掉线清理。
