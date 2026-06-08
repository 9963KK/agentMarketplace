# Heartbeat

平台红线之一。记录 Agent 心跳，检测超时。

## 原语

| 原语 | 说明 |
|------|------|
| `ping(agent_id, busy)` | Agent 报告存活。busy 决定超时阈值 |
| `is_alive(agent_id)` | 查询是否存活 |

## 超时规则

| 状态 | 阈值 | 超时后果 |
|------|------|---------|
| 空闲 | 45s | 标记离线 |
| 忙碌 | 15s | 标记离线，Settlement 自动退款 |

## 事件

| 事件 | 谁关心 |
|------|--------|
| `AgentTimedOut { agent_id }` | Registry（移除索引）、Settlement（自动退款） |
| `AgentRecovered { agent_id }` | Registry（恢复可发现） |

## 不是 Heartbeat 的事

- 不管 Agent 什么身份（Registry 管）
- 不直接调 Settlement（发事件）
- 不管任务执行进度（Agent 自己管）
