# Cookie 固定 15 天有效期设计

## 目标

将 Cookie 的本地过期判定从保存后 24 小时调整为保存后固定 15×24 小时。这里的“半个月”明确解释为固定 15 天，不随自然月长度变化。

## 设计

在 `src-tauri/src/db.rs` 中定义 Cookie 有效天数常量，并由 `Db::is_cookie_expired` 使用 `chrono::Duration::days` 计算过期阈值。

判定规则：

- `cookie_saved_at` 可解析，且保存时间距当前时间不超过 15 天：未过期。
- `cookie_saved_at` 可解析，且保存时间距当前时间超过 15 天：已过期。
- `cookie_saved_at` 缺失、数据库读取失败或时间格式无效：保持现有安全行为，视为已过期。

## 影响范围

仅修改本地 Cookie 过期判断及其测试。不修改数据库结构、既有时间戳、Cookie/API Key 加密格式、IPC command、前端接口或网络请求。

已有 Cookie 自动采用新阈值，无需迁移。

## 测试

为过期判定增加单元测试，覆盖：

1. 保存时间未满 15 天时未过期。
2. 保存时间超过 15 天时已过期。
3. 时间戳缺失时已过期。
4. 时间戳无效时已过期。

测试应避免依赖精确瞬时时钟边界，使用明显位于阈值两侧的时间值。

## 验证

运行：

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
