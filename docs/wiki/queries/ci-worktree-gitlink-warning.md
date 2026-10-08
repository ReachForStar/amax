---
title: CI checkout 后处理警告：误提交的 worktree gitlink
type: query
tags: [CI, git, worktree, 仓库卫生]
created: 2026-10-08
updated: 2026-10-08
status: draft
---

# CI checkout 后处理警告：误提交的 worktree gitlink

## 问题

Test 工作流 Linux 作业（push `93f4362`，运行 37753527059）的 Post Checkout 步骤出现警告，作业本身仍成功：

```
fatal: No url found for submodule path '.claude/worktrees/agent-a09009c6c9572add2' in .gitmodules
##[warning]The process '/usr/bin/git' failed with exit code 128
```

## 根因

`.claude/worktrees/agent-*` 共 12 个条目以 gitlink（mode 160000）形式存在于 master 树中（早期提交 `da2beb3` 带入），仓库没有 `.gitmodules`。actions/checkout 的后处理会执行 `git submodule foreach --recursive` 清理 `core.sshCommand`，遍历 gitlink 时解析不到 url 即 fatal。

对照确认与业务代码无关：上一次 Test（push `9116c5a`，运行 37744308264）无此警告，两者之间仓库没有相关变化；runner 的 git 滚到 2.55.0，新版本对该情形更严格。`.gitignore` 已含 `.claude/` 规则，但已跟踪条目不受忽略规则影响。

## 解法（待用户决定）

本地工作区中这些目录已不存在（`git status` 显示 12 条删除）。清理方式：提交这些删除（`git rm --cached -r .claude/worktrees/`），gitlink 从树中消失、警告消失，`.gitignore` 的 `.claude/` 规则此后生效。属仓库卫生清理，与功能无关。

## 涉及模块

- 仓库根 `.claude/worktrees/`：Claude Code agent 工作树管理目录，不应纳入版本控制
- `.github/workflows/test.yml`：警告出现的作业（Linux 侧 Post Checkout）

## 复发预防

工具生成目录提交前确认忽略规则已生效；对已跟踪文件，`.gitignore` 不追溯。
