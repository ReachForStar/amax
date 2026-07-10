---
name: verify
summary: 启动并验证 AMAX Tauri 桌面窗口、顶部操作栏和最小化托盘行为。
---

# AMAX 运行验证

1. 在仓库根目录运行 `cargo tauri dev`。
2. 等待窗口标题 `AMAX Dashboard` 出现。
3. 截图检查顶部栏的更新时间、刷新和设置按钮。
4. 使用窗口最小化按钮，确认窗口隐藏且 Windows 任务栏不再保留 AMAX 按钮。
5. 展开系统托盘并悬停 AMAX 图标，检查提示包含“今日消耗”和“剩余额度”。
6. 左键单击托盘图标，确认主窗口恢复并聚焦。

Windows 可用 `UIAutomationClient` 检查任务栏按钮是否消失；多显示器截图使用 Pillow `ImageGrab.grab(all_screens=True)`。
