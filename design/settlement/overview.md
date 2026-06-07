# Settlement

结算。**平台红线之一**。市场唯一需要信任的组件。

## 原语

- `hold(from, amount, reference)` — 托管资金，`reference` 通常指向 chain/node
- `release(hold_id, to)` — 放款（需 review 记录）
- `refund(hold_id)` — 退款
- `balance(agent_id)` — 查询余额

## 平台规则

- `release` 前置条件：关联 chain node 有 review 记录
- Agent 掉线 → 自动 `refund`
- 流水只追加，不可删除修改
- 结算流水只引用 chain/node/artifact hash，不保存 artifact 正文

## 待细化

- 托管池隔离
- 自动退款触发机制
- 流水审计
- 与 ChainLedger 的 node 状态联动
