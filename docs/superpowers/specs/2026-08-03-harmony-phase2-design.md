# HarmonyOS 二期设计：统计页 + 导出 + 通知 + 后台刷新

日期：2026-08-03
状态：待评审

## 背景与目标

一期完成配置页（WebView 登录 + 手动粘贴）、看板页、HUKS 加密、断点自适应。二期将桌面版统计功能完整移植到鸿蒙版：

1. **统计页**：消耗趋势 / 请求数趋势 / 模型分布 / 区间汇总 / 余额趋势 + 区间选择器
2. **数据导出**：CSV / JSON（文件保存 + 系统分享）
3. **低余额通知**：剩余额度低于 10% 时系统通知（每次启动周期只发一次）
4. **后台刷新**：返回前台刷新（一期已有）+ WorkScheduler 延迟任务机会性后台刷新

统计方案继承桌面版已确认的 spec（`docs/superpowers/specs/2026-08-02-stats-analytics-design.md`）：金额口径 QUOTA_PER_YUAN=500_000、官方 by-model 任意区间聚合、补零、占比 quota 全 0 切 tokens 口径、快照本地时区 + `date(saved_at,'localtime')`、请求数日末差分、60s 刷新节流等规则全部沿用。

## 关键决策

| 决策点 | 结论 | 理由 |
|--------|------|------|
| 图表实现 | **Canvas 自绘**（折线 + 环图） | ArkUI 无内置图表组件；官方 chart 库能力有限；Canvas 可完全复刻桌面版视觉（色板/mono 数字），零依赖 |
| XLSX 导出 | **不做**，仅 CSV/JSON | SheetJS 无 ArkTS 版本，自实现 XLSX（zip+XML）复杂度高且维护风险大；Excel 可直接打开 CSV |
| 导出交付 | 应用沙箱保存 + 系统分享面板 | HarmonyOS 文件访问受控，走 `picker.DocumentSaveOptions` 保存或 `wantConstant` 分享 |
| 低余额通知 | `@ohos.notificationManager`，权限首次请求 | 与桌面版行为对齐（低于 10%，每周期一次） |
| 后台刷新 | 返回前台刷新（已有）+ `@ohos.resourceschedule.workSchedulerExtensionAbility` 延迟任务 | HarmonyOS 限制：长时任务需特殊权限，延迟任务由系统按条件（充电/空闲等）调度，不保证精确间隔——**无法复刻桌面版 10 分钟精确周期**，属系统约束 |
| 区间选择器 | 预设 7/14/30 + 自定义起止日期（上限 1096 天） | 与桌面版一致 |

## 架构

```
harmony/entry/src/main/ets/
├── pages/
│   └── StatsPage.ets          # 新增：统计页（入口自看板顶栏图表按钮）
├── model/
│   ├── Api.ets                # 扩展：fetchUsageStats(cookie, startDate, endDate)
│   │                          #   by-model 任意区间 + aggregateUsage 聚合（补零/占比/汇总）
│   └── Store.ets              # 扩展：getDailySnapshots(start, end)（date(...,'localtime')）
│                              #   deriveDailyRequests / summarizeLocal（与桌面版 db.rs 同逻辑）
├── common/
│   ├── LineChart.ets          # 新增：Canvas 折线图组件（多序列/双轴/缺口断开/点击提示）
│   ├── DonutChart.ets         # 新增：Canvas 环图组件（占比/中心文本）
│   └── Format.ets             # 扩展：localRfc3339 迁移至此（看板页复用）
├── workscheduler/
│   └── WorkSchedulerExtension.ets  # 新增：延迟任务，触发后台刷新
└── entryability/EntryAbility.ets   # 扩展：低余额通知检查 + workScheduler 启动
```

数据流与容错继承桌面版：官方为主源（失败降级本地估算并标注）、请求数本地差分、余额本地快照、刷新串行化防重复。

## UI（StatsPage）

布局沿用桌面版统计页结构，断点自适应（内容区限宽 720vp 居中）：

```
┌──────────────────────────┐
│ AMAX / ANALYTICS [导出][返回]│  顶栏：导出按钮（CSV/JSON 菜单）+ 返回看板
├──────────────────────────┤
│ [7天][14天][30天] [起]~[止] │  区间选择器
├──────────────────────────┤
│ 累计费用 日均费用 单日峰值   │  汇总卡（自适应网格）
│ 有使用天数 区间总请求数     │
├──────────────────────────┤
│ 消耗趋势（Canvas 折线）     │  费用 ¥ + Tokens 双轴；降级时本地估算虚线
├──────────────────────────┤
│ 请求数趋势（Canvas 折线）   │  差分为空断开
├──────────────────────────┤
│ 模型分布（环图 + 列表）     │  名称/金额/占比/请求数；quota 全 0 标注 Tokens 口径
├──────────────────────────┤
│ 余额趋势（Canvas 折线）     │  本地快照，无数据隐藏
└──────────────────────────┘
```

图表交互：触摸显示数值浮层（Canvas 命中检测）；无图例开关（鸿蒙版简化，序列以固定样式区分）。

## 后台刷新设计

- **返回前台刷新**：一期已实现（DashboardPage onPageShow 60s 节流）；StatsPage 打开时同样触发一次数据加载。
- **WorkScheduler 延迟任务**：应用切后台时申请 `workScheduler.startWork`（条件：默认），系统调度触发 ExtensionAbility → 执行一次刷新（写快照 + 低余额通知检查）→ 完成后重新申请下一次。
- **明确限制**：触发时机由系统决定（可能延迟数十分钟至数小时），非精确周期；看板数据以打开应用时的实时请求为准。

## 低余额通知

- 刷新成功后检查 `percent < 10` 且本启动周期未发过 → `notificationManager.publish`（标题"AMAX 额度不足"，正文含剩余/总额）。
- 权限：`ohos.permission.NOTIFICATION_AGENT` 不需要（普通应用通知仅需用户在设置允许，首次 publish 前经 `notificationManager.isNotificationEnabled` 检查，未开启时静默跳过并记录）。

## 测试

- **本地单元测试**（hypium LocalTest）：aggregateUsage（补零/占比切换/汇总派生）、deriveDailyRequests（相邻/缺档/空值）、CSV 生成文本。
- **仪器测试**：getDailySnapshots 按日查询（'localtime' 口径）、快照差分往返。
- **GUI 验证**：统计页全块渲染、区间切换、降级路径、导出文件内容、通知触发、断点适配。

## 范围外

- XLSX 导出（理由见决策表）
- 精确后台周期刷新（系统限制）
- 图表图例开关/动画（简化项，后续按需）
