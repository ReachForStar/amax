# HarmonyOS 一期实施计划（基础 + 配置 + 看板 + 自适应）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `harmony/` 子目录构建 HarmonyOS 6 原生应用一期：工程脚手架、HUKS 凭据加密、配置页（应用内 WebView 登录 + 手动粘贴）、看板页（官网数据）、断点自适应，端到端可装机使用。

**Architecture:** ArkTS/ArkUI Stage 模型单 entry 模块；页面层（DashboardPage / ConfigPage / LoginPage）+ model 层（Api / Store / Secret）+ common（Theme / BreakpointSystem / GridBackground / Format）；`@ohos.router` 页面导航；HUKS AES-256-GCM 加密 Cookie。

**Tech Stack:** ArkTS、ArkUI、@kit.UniversalKeystoreKit（HUKS）、@kit.ArkData（preferences / relationalStore）、@kit.NetworkKit（http）、@kit.ArkWeb（Web / WebCookieManager）、@kit.ArkUI（router / mediaquery / Canvas）、@ohos/hypium（本地测试）、devecocli v1.2.1 工具链。

## Global Constraints

- API level 23（HarmonyOS 6），bundleName `com.amax.dashboard`，deviceTypes `["phone", "tablet", "2in1"]`。
- 金额口径：`QUOTA_PER_YUAN = 500_000`，`yuan = quota / QUOTA_PER_YUAN`（与桌面版一致）。
- 请求协议与桌面版对齐：headers `User-Agent: Mozilla/5.0` / `Accept: application/json` / `x-company: AMAX` / `Cookie`；by-model body 含本地时区当日 `start_time` / `end_time`（Unix 秒）、字符串 `user_id`、`status: "success"`；日志失败仍返回额度（`logs_available=false`）。
- 错误分级：401/403 → "Cookie 无效或已过期，请重新登录"；429 → "请求过于频繁，请稍后重试"；连接失败 → "网络连接失败，请检查网络"。
- 已废弃 API 禁止使用：模块级 `mediaquery.matchMediaSync`（API 18 废弃，用 `UIContext.getMediaQuery().matchMediaSync`）、`onUrlLoadIntercept`（用 `onLoadIntercept`）、`getCookie/setCookie`（用 `fetchCookie/configCookie`）。
- 所有 ArkTS API 细节实施前经 `/c/nvm4w/nodejs/devecocli docs read "<documentId>"` 复核（各任务给出 documentId），与文档不符时以文档为准并在报告中记录偏差。
- 快照 `saved_at` 本地时区 RFC3339；子项目 2 按日查询沿用 `date(saved_at, 'localtime')`（规避桌面版时区缺陷）。
- 提交信息中文祈使句，主题 ≤50 字符；`git add` 仅具体文件；不 push。
- 注释简体中文。
- 构建验证统一用 `/c/nvm4w/nodejs/devecocli build`（在 harmony/ 目录执行）；本地单元测试用 hvigorw（Task 3 给出命令）。

## File Structure

```
harmony/                                 # devecocli create 生成，API 23
├── AppScope/
│   └── app.json5                        # bundleName: com.amax.dashboard
├── entry/src/
│   ├── main/
│   │   ├── ets/
│   │   │   ├── entryability/EntryAbility.ets   # 窗口 + 初始路由（有 Cookie→Dashboard，否则 Config）
│   │   │   ├── pages/
│   │   │   │   ├── DashboardPage.ets    # Task 8
│   │   │   │   ├── ConfigPage.ets       # Task 7
│   │   │   │   └── LoginPage.ets        # Task 7（WebView 登录）
│   │   │   ├── model/
│   │   │   │   ├── Secret.ets           # Task 4（HUKS）
│   │   │   │   ├── Store.ets            # Task 5（preferences + RDB）
│   │   │   │   └── Api.ets              # Task 6（http）
│   │   │   └── common/
│   │   │       ├── Theme.ets            # Task 2
│   │   │       ├── BreakpointSystem.ets # Task 2
│   │   │       ├── GridBackground.ets   # Task 2（Canvas 网格背景）
│   │   │       └── Format.ets           # Task 3（纯函数 + 错误分级）
│   │   ├── resources/                   # 图标 / 字符串
│   │   └── module.json5                 # INTERNET 权限、deviceTypes、页面注册
│   ├── test/                            # Task 3 本地单元测试（@ohos/hypium）
│   └── ohosTest/                        # 仪器测试（Task 9，需设备）
├── build-profile.json5                  # 签名配置（Task 9 devecocli signature generate）
└── oh-package.json5
```

---

### Task 1: 工程脚手架

**Files:**
- Create: `harmony/`（devecocli create 生成的完整工程）
- Modify: 根目录 `.gitignore`（追加 harmony 构建产物）

**Interfaces:**
- Produces: 可编译的空 Stage 工程；后续所有任务在其上修改。

- [ ] **Step 1: 生成工程**

```bash
cd D:/file/rs-project/amax
/c/nvm4w/nodejs/devecocli create --app-name AmaxDashboard --project-path harmony --bundle-name com.amax.dashboard --api-level 23
```

Expected: harmony/ 目录生成，含 AppScope / entry / build-profile.json5 / oh-package.json5。

- [ ] **Step 2: 验证模板可编译**

```bash
cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build
```

Expected: BUILD SUCCESSFUL（首次构建下载依赖可能较慢）。失败时查 `devecocli build` 输出定位；环境类错误（SDK 路径等）报 BLOCKED。

- [ ] **Step 3: 记录模板结构**

浏览生成的模板，确认以下文件位置并在报告中记录实际路径（模板版本差异）：
- `entry/src/main/ets/pages/Index.ets`（模板首页，Task 8 将替换为 DashboardPage）
- `entry/src/main/module.json5`（abilities / pages 注册位置）
- `entry/src/main/resources/base/profile/main_pages.json`（页面路由表）
- `entry/src/test/List.test.ets`（测试注册入口，Task 3 使用）

- [ ] **Step 4: 追加 .gitignore 规则**

在根目录 `.gitignore` 末尾追加：

```
# HarmonyOS
harmony/**/build/
harmony/.hvigor/
harmony/**/oh_modules/
harmony/.idea/
harmony/**/*.har
```

- [ ] **Step 5: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony .gitignore
git commit -m "feat: 初始化 HarmonyOS 工程脚手架"
```

---

### Task 2: 主题常量、断点系统与网格背景

**Files:**
- Create: `harmony/entry/src/main/ets/common/Theme.ets`
- Create: `harmony/entry/src/main/ets/common/BreakpointSystem.ets`
- Create: `harmony/entry/src/main/ets/common/GridBackground.ets`

**Interfaces:**
- Produces:
  - `Theme` 常量对象（色板 / 字体 / 间距）
  - `BreakpointSystem.register(uiContext)` / `unregister()`（写入 AppStorage `'currentBreakpoint'`：`'sm' | 'md' | 'lg'`）
  - `GridBackground` @Component（Canvas 网格背景层）
- Consumes: 无

**Docs（实施前复核）:**
- 断点：`devecocli docs read "最佳实践/多设备界面开发/界面布局响应式变化/响应式布局/bpta-multi-device-responsive-layout"` 与 `devecocli docs read "API参考/ArkUI_方舟UI框架/ArkTS_API/UI界面/ohos_mediaquery_媒体查询_/js-apis-mediaquery"`
- Canvas：`devecocli docs read "开发指南/ArkUI_方舟UI框架/UI开发_ArkTS声明式开发范式/使用自定义能力/自定义绘制/使用画布绘制自定义图形_Canvas/arkts-drawing-customization-on-canvas"`

- [ ] **Step 1: Theme.ets**

```typescript
// 设计常量：桌面版色板逐值移植
export class Theme {
  static readonly surface: string = '#e8ece8';
  static readonly surfaceRaised: string = '#f2f4f1';
  static readonly ink: string = '#19221e';
  static readonly muted: string = '#5a6460';
  static readonly line: string = '#bec9c1';
  static readonly accent: string = '#2c6b4b';
  static readonly accentSoft: string = '#d2e2d7';
  static readonly error: string = '#8b3e3e';
  static readonly errorBg: string = '#eadada';
  static readonly fontMono: string = 'monospace';
  // 网格背景色（桌面版同值）
  static readonly gridH: string = 'rgba(44,107,75,0.045)';
  static readonly gridV: string = 'rgba(44,107,75,0.035)';
  static readonly gridSize: number = 24;
  // 断点自适应：内容区最大宽度与页面边距
  static contentMaxWidth(bp: string): number {
    return bp === 'sm' ? 10000 : 720; // sm 满宽；md/lg 限宽居中
  }
  static pagePadding(bp: string): number {
    return bp === 'sm' ? 24 : 40;
  }
}
```

- [ ] **Step 2: BreakpointSystem.ets**

断点定义采用官方区间：sm [320,600)、md [600,840)、lg [840,∞) vp。

```typescript
import { mediaquery } from '@kit.ArkUI';

// 窗口宽度断点系统：监听结果写入 AppStorage 'currentBreakpoint'
// 页面经 @StorageProp('currentBreakpoint') 订阅
export class BreakpointSystem {
  private smListener: mediaquery.MediaQueryListener | null = null;
  private mdListener: mediaquery.MediaQueryListener | null = null;
  private lgListener: mediaquery.MediaQueryListener | null = null;

  register(uiContext: UIContext): void {
    const mq = uiContext.getMediaQuery();
    this.smListener = mq.matchMediaSync('(320vp<=width<600vp)');
    this.smListener.on('change', (result: mediaquery.MediaQueryResult) => {
      if (result.matches) {
        AppStorage.setOrCreate('currentBreakpoint', 'sm');
      }
    });
    this.mdListener = mq.matchMediaSync('(600vp<=width<840vp)');
    this.mdListener.on('change', (result: mediaquery.MediaQueryResult) => {
      if (result.matches) {
        AppStorage.setOrCreate('currentBreakpoint', 'md');
      }
    });
    this.lgListener = mq.matchMediaSync('(840vp<=width)');
    this.lgListener.on('change', (result: mediaquery.MediaQueryResult) => {
      if (result.matches) {
        AppStorage.setOrCreate('currentBreakpoint', 'lg');
      }
    });
    // 初始值：按当前匹配状态取一档
    if (this.lgListener.matches) {
      AppStorage.setOrCreate('currentBreakpoint', 'lg');
    } else if (this.mdListener.matches) {
      AppStorage.setOrCreate('currentBreakpoint', 'md');
    } else {
      AppStorage.setOrCreate('currentBreakpoint', 'sm');
    }
  }

  unregister(): void {
    this.smListener?.off('change');
    this.mdListener?.off('change');
    this.lgListener?.off('change');
  }
}
```

- [ ] **Step 3: GridBackground.ets**

```typescript
import { Theme } from './Theme';

// 桌面版品牌网格背景：Canvas 绘制 24vp 间距网格线
@Component
export struct GridBackground {
  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private context: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);

  build() {
    Canvas(this.context)
      .width('100%')
      .height('100%')
      .onReady(() => {
        const w = this.context.width;
        const h = this.context.height;
        const step = Theme.gridSize;
        this.context.lineWidth = 1;
        // 横线
        this.context.strokeStyle = Theme.gridH;
        this.context.beginPath();
        for (let y = 0; y <= h; y += step) {
          this.context.moveTo(0, y);
          this.context.lineTo(w, y);
        }
        this.context.stroke();
        // 纵线
        this.context.strokeStyle = Theme.gridV;
        this.context.beginPath();
        for (let x = 0; x <= w; x += step) {
          this.context.moveTo(x, 0);
          this.context.lineTo(x, h);
        }
        this.context.stroke();
      })
  }
}
```

- [ ] **Step 4: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。

- [ ] **Step 5: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/common/Theme.ets harmony/entry/src/main/ets/common/BreakpointSystem.ets harmony/entry/src/main/ets/common/GridBackground.ets
git commit -m "feat: 主题常量与断点自适应基础"
```

---

### Task 3: 纯函数模块与本地单元测试（TDD）

**Files:**
- Create: `harmony/entry/src/main/ets/common/Format.ets`
- Create: `harmony/entry/src/test/Format.test.ets`
- Modify: `harmony/entry/src/test/List.test.ets`（注册用例）

**Interfaces:**
- Produces:
  - `formatTokens(n: number): string`（≥1e6 → `x.xxM`；≥1e3 → `x.xK`；否则原值字符串）
  - `formatYuan(n: number): string`（`¥` + 6 位小数）
  - `parseSessionCookie(cookieStr: string): string | null`（从整串 Cookie 提取 `session` 值）
  - `classifyHttpError(code: number): string`（错误分级文案）
  - `class ApiError extends Error`（携带分级消息）
- Consumes: 无

**Docs:** 本地测试：`devecocli docs read "开发指南/开发自测试/测试框架/代码测试/Local_Test/ide-local-test"`

- [ ] **Step 1: 写失败测试**

`harmony/entry/src/test/Format.test.ets`：

```typescript
import { describe, it, expect } from '@ohos/hypium';
import { formatTokens, formatYuan, parseSessionCookie, classifyHttpError } from '../main/ets/common/Format';

export default function formatTest() {
  describe('formatTest', () => {
    it('formatTokens_millions', 0, () => {
      expect(formatTokens(89936759)).assertEqual('89.94M');
      expect(formatTokens(1000000)).assertEqual('1.00M');
    });
    it('formatTokens_thousands', 0, () => {
      expect(formatTokens(26072)).assertEqual('26.1K');
      expect(formatTokens(1000)).assertEqual('1.0K');
    });
    it('formatTokens_plain', 0, () => {
      expect(formatTokens(999)).assertEqual('999');
      expect(formatTokens(0)).assertEqual('0');
    });
    it('formatYuan_six_decimals', 0, () => {
      expect(formatYuan(0.014014)).assertEqual('¥0.014014');
      expect(formatYuan(0)).assertEqual('¥0.000000');
    });
    it('parseSessionCookie_extracts_value', 0, () => {
      expect(parseSessionCookie('a=1; session=MTc4MzQy; b=2')).assertEqual('MTc4MzQy');
      expect(parseSessionCookie('session=only')).assertEqual('only');
    });
    it('parseSessionCookie_missing_or_empty', 0, () => {
      expect(parseSessionCookie('a=1; b=2')).assertNull();
      expect(parseSessionCookie('')).assertNull();
      expect(parseSessionCookie('session=')).assertNull();
    });
    it('classifyHttpError_tiers', 0, () => {
      expect(classifyHttpError(401)).assertEqual('Cookie 无效或已过期，请重新登录');
      expect(classifyHttpError(403)).assertEqual('Cookie 无效或已过期，请重新登录');
      expect(classifyHttpError(429)).assertEqual('请求过于频繁，请稍后重试');
      expect(classifyHttpError(500)).assertEqual('服务异常（HTTP 500）');
    });
  });
}
```

在 `entry/src/test/List.test.ets` 注册（模板已有示例注册行，追加）：

```typescript
import formatTest from './Format.test';

export default function testsuite() {
  formatTest();
}
```

（保留模板既有其他注册行；若模板示例行引用的文件不存在则一并清理。）

- [ ] **Step 2: 运行测试确认失败**

```bash
cd D:/file/rs-project/amax/harmony
hvigorw test -p module=entry -p product=default -p testType=LocalTest --no-daemon
```

Expected: 编译失败（Format.ets 不存在）。若 `hvigorw` 不在 PATH，用 `D:/command-line-tools/bin/hvigorw.bat`；命令参数不兼容时经 `devecocli docs read "开发指南/开发自测试/测试框架/代码测试/Local_Test/ide-local-test"` 查证正确形式。

- [ ] **Step 3: 实现 Format.ets**

```typescript
// 纯函数：Token 与金额格式化、Cookie 解析、HTTP 错误分级
export function formatTokens(n: number): string {
  if (n >= 1e6) {
    return (n / 1e6).toFixed(2) + 'M';
  }
  if (n >= 1e3) {
    return (n / 1e3).toFixed(1) + 'K';
  }
  return String(n);
}

export function formatYuan(n: number): string {
  return '¥' + n.toFixed(6);
}

// 从整串 Cookie（"a=1; session=xxx; b=2"）提取 session 值
export function parseSessionCookie(cookieStr: string): string | null {
  if (!cookieStr) {
    return null;
  }
  const match = cookieStr.match(/(?:^|;\s*)session=([^;]+)/);
  if (!match || match[1].length === 0) {
    return null;
  }
  return match[1];
}

export function classifyHttpError(code: number): string {
  if (code === 401 || code === 403) {
    return 'Cookie 无效或已过期，请重新登录';
  }
  if (code === 429) {
    return '请求过于频繁，请稍后重试';
  }
  return `服务异常（HTTP ${code}）`;
}

// API 层统一错误：message 为分级后的用户可见文案
export class ApiError extends Error {
  readonly isAuthError: boolean;

  constructor(message: string, isAuthError: boolean = false) {
    super(message);
    this.isAuthError = isAuthError;
  }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd D:/file/rs-project/amax/harmony && hvigorw test -p module=entry -p product=default -p testType=LocalTest --no-daemon`
Expected: 全部用例 PASS。

- [ ] **Step 5: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/common/Format.ets harmony/entry/src/test/Format.test.ets harmony/entry/src/test/List.test.ets
git commit -m "feat: 格式化工具与本地单元测试"
```

---

### Task 4: HUKS 凭据加密（Secret.ets）

**Files:**
- Create: `harmony/entry/src/main/ets/model/Secret.ets`

**Interfaces:**
- Produces:
  - `encryptSecret(plain: string): Promise<string>` → `huks:v1:<base64(iv|密文|tag)>`
  - `decryptSecret(stored: string): Promise<string>`（无前缀的旧明文原样返回；解密失败抛 Error）
  - 密钥别名 `amax_secret_key`，AES-256-GCM，首次使用懒生成（幂等）
- Consumes: 无

**Docs（实施前必查，以文档为准修正参数）:**
- `devecocli docs read "API参考/安全/Universal_Keystore_Kit_密钥管理服务/ArkTS_API/ohos_security_huks_通用密钥库系统_/js-apis-huks"`
- `devecocli docs read "FAQ/安全/密钥管理_Universal_Keystore/HUKS如何分段加解密/faqs-universal-keystore-15"`
- 密钥存在性检查 API（isKeyItemExist）与随机数（cryptoFramework.createRandom）一并查证

- [ ] **Step 1: 实现 Secret.ets**

```typescript
import { huks } from '@kit.UniversalKeystoreKit';
import { cryptoFramework } from '@kit.CryptoArchitectureKit';
import { util } from '@kit.ArkTS';

const KEY_ALIAS = 'amax_secret_key';
const PREFIX = 'huks:v1:';
const IV_LENGTH = 12;

function baseProperties(purpose: huks.HuksKeyPurpose, nonce: Uint8Array | null): Array<huks.HuksParam> {
  const props: Array<huks.HuksParam> = [
    { tag: huks.HuksTag.HUKS_TAG_ALGORITHM, value: huks.HuksKeyAlg.HUKS_ALG_AES },
    { tag: huks.HuksTag.HUKS_TAG_KEY_SIZE, value: huks.HuksKeySize.HUKS_AES_KEY_SIZE_256 },
    { tag: huks.HuksTag.HUKS_TAG_PURPOSE, value: purpose },
    { tag: huks.HuksTag.HUKS_TAG_PADDING, value: huks.HuksKeyPadding.HUKS_PADDING_NONE },
    { tag: huks.HuksTag.HUKS_TAG_BLOCK_MODE, value: huks.HuksCipherMode.HUKS_MODE_GCM },
  ];
  if (nonce !== null) {
    props.push({ tag: huks.HuksTag.HUKS_TAG_NONCE, value: nonce });
    props.push({ tag: huks.HuksTag.HUKS_TAG_ASSOCIATED_DATA, value: new Uint8Array(0) });
  }
  return props;
}

const ENCRYPT_DECRYPT =
  huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_ENCRYPT | huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_DECRYPT;

// 密钥懒生成：generateKeyItem 对同名密钥的行为经 docs 查证；
// 若为"已存在即报错"，则先 isKeyItemExist 判断（查证后二选一实现）
async function ensureKey(): Promise<void> {
  const exists = await huks.isKeyItemExist(KEY_ALIAS, true);
  if (!exists) {
    await huks.generateKeyItem(KEY_ALIAS, { properties: baseProperties(ENCRYPT_DECRYPT, null) });
  }
}

function randomBytes(length: number): Uint8Array {
  const random = cryptoFramework.createRandom();
  return random.generateRandomSync(length).randomNumber;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

// 三段式 GCM 会话：init → update（携带数据）→ finish（inData 置空）
async function runSession(purpose: huks.HuksKeyPurpose, nonce: Uint8Array, inData: Uint8Array): Promise<Uint8Array> {
  const options: huks.HuksOptions = {
    properties: baseProperties(purpose, nonce),
    inData: inData,
  };
  const init = await huks.initSession(KEY_ALIAS, options);
  const update = await huks.updateSession(init.handle, options);
  options.inData = new Uint8Array(0);
  const finish = await huks.finishSession(init.handle, options);
  return concat(update.outData, finish.outData);
}

export async function encryptSecret(plain: string): Promise<string> {
  await ensureKey();
  const iv = randomBytes(IV_LENGTH);
  const plainBytes = new util.TextEncoder().encodeInto(plain);
  const cipher = await runSession(huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_ENCRYPT, iv, plainBytes);
  // 存储格式：iv(12B) | 密文 | GCM tag（HUKS 已将 tag 附于密文末尾）
  const base64 = new util.Base64Helper();
  return PREFIX + base64.encodeToStringSync(concat(iv, cipher));
}

export async function decryptSecret(stored: string): Promise<string> {
  // 旧明文兼容：无前缀原样返回
  if (!stored.startsWith(PREFIX)) {
    return stored;
  }
  await ensureKey();
  const blob = new util.Base64Helper().decodeSync(stored.slice(PREFIX.length));
  const iv = blob.slice(0, IV_LENGTH);
  const cipher = blob.slice(IV_LENGTH);
  const out = await runSession(huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_DECRYPT, iv, cipher);
  return util.TextDecoder.create('utf-8').decodeToString(out);
  // 解密失败（篡改/密钥异常）由 HUKS 抛错，调用方视为无效凭据，不在此吞错
}
```

- [ ] **Step 2: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。API 签名与文档不符时按 docs read 结果修正并在报告中记录偏差（HUKS 属系统 API，本地无法运行验证，运行验证在 Task 9）。

- [ ] **Step 3: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/model/Secret.ets
git commit -m "feat: HUKS 凭据加密存储"
```

---

### Task 5: 存储层（Store.ets）

**Files:**
- Create: `harmony/entry/src/main/ets/model/Store.ets`

**Interfaces:**
- Produces:
  - `initStore(context: Context): Promise<void>`（EntryAbility 启动时调用一次）
  - `getCookie(): Promise<string | null>` / `saveCookie(plain: string, source: string): Promise<void>`
  - `getApiKey(): Promise<string | null>` / `saveApiKey(plain: string): Promise<void>`
  - `getCookieSource(): Promise<string>`（默认 `'manual'`）
  - `clearCredentials(): Promise<void>`
  - `saveSnapshot(row: SnapshotRow): Promise<void>`
  - `interface SnapshotRow { savedAt: string; todayYuan: number; totalTokens: number; remaining: number; used: number; total: number; requestCount: number }`
- Consumes: Task 4 `encryptSecret` / `decryptSecret`

**Docs:**
- `devecocli docs read "API参考/ArkData_方舟数据管理/ArkTS_API/ohos_data_preferences_用户首选项_/js-apis-data-preferences"`
- `devecocli docs read "API参考/ArkData_方舟数据管理/ArkTS_API/已停止维护的接口/ohos_data_rdb_关系型数据库_/js-apis-data-rdb"`（方法签名同 relationalStore，import 用 `@kit.ArkData` 的 `relationalStore`）

- [ ] **Step 1: 实现 Store.ets**

```typescript
import { preferences, relationalStore } from '@kit.ArkData';
import { encryptSecret, decryptSecret } from './Secret';

const CONFIG_STORE = 'config';
const DB_NAME = 'amax_dashboard.db';
const KEY_COOKIE = 'cookie';
const KEY_API_KEY = 'api_key';
const KEY_COOKIE_SOURCE = 'cookie_source';

export interface SnapshotRow {
  savedAt: string;        // 本地时区 RFC3339
  todayYuan: number;
  totalTokens: number;
  remaining: number;
  used: number;
  total: number;
  requestCount: number;
}

let configStore: preferences.Preferences | null = null;
let db: relationalStore.RdbStore | null = null;

function requireConfig(): preferences.Preferences {
  if (!configStore) {
    throw new Error('Store 未初始化：先调用 initStore');
  }
  return configStore;
}

function requireDb(): relationalStore.RdbStore {
  if (!db) {
    throw new Error('Store 未初始化：先调用 initStore');
  }
  return db;
}

export async function initStore(context: Context): Promise<void> {
  configStore = await preferences.getPreferences(context, CONFIG_STORE);
  const config: relationalStore.StoreConfig = { name: DB_NAME };
  db = await relationalStore.getRdbStore(context, config);
  // schema 与桌面版 db.rs 逐字段一致
  await db.executeSql(
    `CREATE TABLE IF NOT EXISTS dashboard_snapshot (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      saved_at TEXT NOT NULL,
      today_yuan REAL NOT NULL,
      total_tokens INTEGER NOT NULL,
      remaining REAL NOT NULL,
      used REAL NOT NULL,
      total REAL NOT NULL,
      request_count INTEGER
    )`
  );
}

export async function getCookie(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_COOKIE, '') as string;
  if (stored.length === 0) {
    return null;
  }
  return decryptSecret(stored);
}

export async function saveCookie(plain: string, source: string): Promise<void> {
  const store = requireConfig();
  await store.put(KEY_COOKIE, await encryptSecret(plain));
  await store.put(KEY_COOKIE_SOURCE, source);
  await store.flush();
}

export async function getApiKey(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_API_KEY, '') as string;
  if (stored.length === 0) {
    return null;
  }
  return decryptSecret(stored);
}

export async function saveApiKey(plain: string): Promise<void> {
  const store = requireConfig();
  await store.put(KEY_API_KEY, await encryptSecret(plain));
  await store.flush();
}

export async function getCookieSource(): Promise<string> {
  return await requireConfig().get(KEY_COOKIE_SOURCE, 'manual') as string;
}

export async function clearCredentials(): Promise<void> {
  const store = requireConfig();
  await store.delete(KEY_COOKIE);
  await store.delete(KEY_API_KEY);
  await store.delete(KEY_COOKIE_SOURCE);
  await store.flush();
}

export async function saveSnapshot(row: SnapshotRow): Promise<void> {
  const values: relationalStore.ValuesBucket = {
    'saved_at': row.savedAt,
    'today_yuan': row.todayYuan,
    'total_tokens': row.totalTokens,
    'remaining': row.remaining,
    'used': row.used,
    'total': row.total,
    'request_count': row.requestCount,
  };
  const rowId = await requireDb().insert('dashboard_snapshot', values);
  if (rowId < 0) {
    throw new Error('快照写入失败');
  }
}
```

- [ ] **Step 2: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。

- [ ] **Step 3: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/model/Store.ets
git commit -m "feat: 配置与快照存储层"
```

---

### Task 6: 网络层（Api.ets）

**Files:**
- Create: `harmony/entry/src/main/ets/model/Api.ets`

**Interfaces:**
- Produces:
  - `fetchDashboard(cookie: string): Promise<DashboardData>`
  - `interface DashboardData { displayName: string; requestCount: number; todayYuan: number; todayTokens: number; todayInput: number; todayOutput: number; remaining: number; used: number; total: number; percent: number; logsAvailable: boolean }`
  - 抛 `ApiError`（Task 3），`isAuthError` 标记 401/403
- Consumes: Task 3 `ApiError` / `classifyHttpError`

**Docs:** `devecocli docs read "API参考/网络/Network_Kit_网络服务/ArkTS_API/ohos_net_http_数据请求_/js-apis-http"`

- [ ] **Step 1: 实现 Api.ets**

```typescript
import { http } from '@kit.NetworkKit';
import { ApiError, classifyHttpError } from '../common/Format';

const BASE_URL = 'https://ai.amaxsmp.com';
const QUOTA_PER_YUAN = 500_000;
const TIMEOUT_MS = 30_000;

export interface DashboardData {
  displayName: string;
  requestCount: number;
  todayYuan: number;
  todayTokens: number;
  todayInput: number;
  todayOutput: number;
  remaining: number;
  used: number;
  total: number;
  percent: number;
  logsAvailable: boolean;
}

interface UserInfo {
  id: number;
  quota: number;
  used_quota: number;
  request_count: number;
  display_name: string;
}

interface UsageSummary {
  quota: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
}

function commonHeaders(cookie: string): Record<string, string> {
  return {
    'User-Agent': 'Mozilla/5.0',
    'Accept': 'application/json',
    'x-company': 'AMAX',
    'Cookie': cookie,
  };
}

// 单次请求封装：http 对象一次性使用，完成即 destroy
function request(url: string, cookie: string, method: http.RequestMethod,
  body?: object): Promise<http.HttpResponse> {
  return new Promise((resolve, reject) => {
    const req = http.createHttp();
    req.request(url, {
      method: method,
      header: commonHeaders(cookie),
      extraData: body === undefined ? undefined : JSON.stringify(body),
      connectTimeout: TIMEOUT_MS,
      readTimeout: TIMEOUT_MS,
    }, (err, data: http.HttpResponse) => {
      req.destroy();
      if (err) {
        reject(new ApiError('网络连接失败，请检查网络'));
        return;
      }
      resolve(data);
    });
  });
}

function parseBody<T>(resp: http.HttpResponse): T {
  const text = typeof resp.result === 'string' ? resp.result : JSON.stringify(resp.result);
  return JSON.parse(text) as T;
}

// 本地时区当日起止 Unix 秒（与桌面版 api.rs 同口径）
function todayRange(): { start: number; end: number } {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59, 999);
  return { start: Math.floor(start.getTime() / 1000), end: Math.floor(end.getTime() / 1000) };
}

export async function fetchDashboard(cookie: string): Promise<DashboardData> {
  if (cookie.length === 0) {
    throw new ApiError('Cookie 无效或已过期，请重新登录', true);
  }
  // 1. 账户信息
  const userResp = await request(`${BASE_URL}/api/user/self`, cookie, http.RequestMethod.GET);
  if (userResp.responseCode !== 200) {
    throw new ApiError(classifyHttpError(userResp.responseCode),
      userResp.responseCode === 401 || userResp.responseCode === 403);
  }
  const user = (parseBody<{ data: UserInfo }>(userResp)).data;
  const totalQuota = user.quota + user.used_quota;
  const remaining = user.quota / QUOTA_PER_YUAN;
  const used = user.used_quota / QUOTA_PER_YUAN;
  const total = totalQuota / QUOTA_PER_YUAN;
  const percent = totalQuota > 0
    ? Math.min(100, Math.max(0, user.quota / totalQuota * 100))
    : 0;

  // 2. 当日用量汇总：失败仍返回额度（logsAvailable=false），与桌面版容错一致
  let summary: UsageSummary = { quota: 0, total_tokens: 0, input_tokens: 0, output_tokens: 0 };
  let logsAvailable = false;
  try {
    const range = todayRange();
    const logsResp = await request(`${BASE_URL}/v1/logs/token-usage/by-model`, cookie,
      http.RequestMethod.POST, {
        start_time: range.start,
        end_time: range.end,
        user_id: String(user.id),
        status: 'success',
      });
    if (logsResp.responseCode === 200) {
      const payload = parseBody<{ summary: UsageSummary }>(logsResp);
      if (payload.summary) {
        summary = payload.summary;
        logsAvailable = true;
      }
    } else {
      console.error(`日志汇总异常：HTTP ${logsResp.responseCode}，保留账户额度数据`);
    }
  } catch (error) {
    console.error(`日志汇总失败，保留账户额度数据：${JSON.stringify(error)}`);
  }

  return {
    displayName: user.display_name,
    requestCount: user.request_count,
    todayYuan: summary.quota / QUOTA_PER_YUAN,
    todayTokens: summary.total_tokens,
    todayInput: summary.input_tokens,
    todayOutput: summary.output_tokens,
    remaining,
    used,
    total,
    percent,
    logsAvailable,
  };
}
```

- [ ] **Step 2: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。

- [ ] **Step 3: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/model/Api.ets
git commit -m "feat: 官网数据网络层"
```

---

### Task 7: 配置页与 WebView 登录页

**Files:**
- Create: `harmony/entry/src/main/ets/pages/ConfigPage.ets`
- Create: `harmony/entry/src/main/ets/pages/LoginPage.ets`

**Interfaces:**
- Produces: 两个页面文件（路由表注册见 Task 9）。
- Consumes: Task 2 Theme；Task 3 Format；Task 5 Store；Task 6 Api。

**Docs:**
- `devecocli docs read "API参考/ArkWeb_方舟Web/ArkTS_API/ohos_web_webview_Webview_/Class_WebCookieManager/arkts-apis-webview-webcookiemanager"`
- `devecocli docs read "API参考/ArkWeb_方舟Web/ArkTS_组件/Web/事件/arkts-basic-components-web-events"`
- `devecocli docs read "开发指南/ArkUI_方舟UI框架/UI开发_ArkTS声明式开发范式/设置组件导航和页面路由/页面路由_ohos_router_不推荐/arkts-routing"`

**关键设计:** 存储**完整 cookie 串**（`session=xxx; other=yyy`，请求头需要完整串）；`parseSessionCookie` 仅用于判定登录成功。登录成功导航：`router.clear()` + `router.replaceUrl(DashboardPage)`（清空栈避免回退到登录页）。

- [ ] **Step 1: LoginPage.ets**

```typescript
import { webview } from '@kit.ArkWeb';
import { router } from '@kit.ArkUI';
import { Theme } from '../common/Theme';
import { parseSessionCookie } from '../common/Format';
import { saveCookie } from '../model/Store';

const LOGIN_URL = 'https://ai.amaxsmp.com/login';
const DASHBOARD_PREFIX = 'https://ai.amaxsmp.com/dashboard';

@Entry
@Component
struct LoginPage {
  private controller: webview.WebviewController = new webview.WebviewController();
  @State notice: string = '';
  private handled: boolean = false;

  async onLoginSuccess(): Promise<void> {
    const cookieStr = webview.WebCookieManager.fetchCookieSync('https://ai.amaxsmp.com');
    const session = parseSessionCookie(cookieStr);
    if (session === null) {
      // 已重定向到 dashboard 但未提取到 session：视为登录未完成
      this.notice = '未能提取登录凭据，请重试或返回手动粘贴';
      this.handled = false;
      return;
    }
    await saveCookie(cookieStr, 'webview');
    router.clear();
    router.replaceUrl({ url: 'pages/DashboardPage' });
  }

  build() {
    Column() {
      Row() {
        Text('← 返回').fontSize(15).fontColor(Theme.muted)
          .onClick(() => router.back())
          .padding({ right: 12 })
        Text('登录 AMAX 账户').fontSize(15).fontWeight(FontWeight.Medium).fontColor(Theme.ink)
        Blank()
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 12, bottom: 12 })

      if (this.notice.length > 0) {
        Text(this.notice)
          .fontSize(12).fontColor(Theme.error)
          .backgroundColor(Theme.errorBg)
          .width('100%').padding(10)
      }

      Web({ src: LOGIN_URL, controller: this.controller })
        .width('100%')
        .layoutWeight(1)
        .onLoadIntercept((event) => {
          // 每次 URL 加载前触发；捕获登录后到 /dashboard 的重定向
          const url = event.data.getRequestUrl();
          if (!this.handled && url.startsWith(DASHBOARD_PREFIX)) {
            this.handled = true;
            this.onLoginSuccess();
          }
          return false; // 放行导航
        })
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
```

- [ ] **Step 2: ConfigPage.ets**

```typescript
import { router } from '@kit.ArkUI';
import { Theme } from '../common/Theme';
import { GridBackground } from '../common/GridBackground';
import { ApiError } from '../common/Format';
import { getCookie, saveCookie, saveApiKey, clearCredentials } from '../model/Store';
import { fetchDashboard } from '../model/Api';

@Entry
@Component
struct ConfigPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State cookieInput: string = '';
  @State apiKeyInput: string = '';
  @State statusText: string = '';
  @State statusIsError: boolean = false;
  @State busy: boolean = false;
  @State hasCookie: boolean = false;

  async aboutToAppear(): Promise<void> {
    this.hasCookie = (await getCookie()) !== null;
  }

  setStatus(text: string, isError: boolean): void {
    this.statusText = text;
    this.statusIsError = isError;
  }

  async saveManual(): Promise<void> {
    const cookie = this.cookieInput.trim();
    const apiKey = this.apiKeyInput.trim();
    if (cookie.length === 0 && apiKey.length === 0) {
      this.setStatus('请至少填写一项认证信息', true);
      return;
    }
    this.busy = true;
    try {
      if (cookie.length > 0) {
        await saveCookie(cookie, 'manual');
      }
      if (apiKey.length > 0) {
        await saveApiKey(apiKey);
      }
      // 保存后立即验证
      await fetchDashboard((await getCookie()) ?? '');
      router.clear();
      router.replaceUrl({ url: 'pages/DashboardPage' });
    } catch (error) {
      const message = error instanceof ApiError ? error.message : '保存失败，请重试';
      this.setStatus(message, true);
      this.busy = false;
    }
  }

  async relogin(): Promise<void> {
    await clearCredentials();
    this.hasCookie = false;
    this.setStatus('', false);
    router.pushUrl({ url: 'pages/LoginPage' });
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      GridBackground()
      Scroll() {
        Column({ space: 22 }) {
          Text('AMAX / ACCESS')
            .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
            .fontColor(Theme.accent).letterSpacing(2)
          Text('连接你的 AMAX 账户').fontSize(26).fontWeight(FontWeight.Bold)
            .fontColor(Theme.ink).margin({ top: 10 })
          Text('凭据只保存在当前设备的本地应用数据中（HUKS 加密）。')
            .fontSize(13).fontColor(Theme.muted)

          if (this.hasCookie) {
            Button('重新登录（清除现有凭据）')
              .width('100%').height(44)
              .backgroundColor(Theme.surfaceRaised).fontColor(Theme.accent)
              .border({ width: 1, color: Theme.accent, radius: 4 })
              .onClick(() => this.relogin())
          }

          Button('前往登录（推荐）')
            .width('100%').height(48)
            .backgroundColor(Theme.accent).fontColor('#f7faf7')
            .fontSize(15).fontWeight(FontWeight.Medium).borderRadius(4)
            .enabled(!this.busy)
            .onClick(() => router.pushUrl({ url: 'pages/LoginPage' }))

          Column({ space: 10 }) {
            Text('或手动粘贴 Session Cookie')
              .fontSize(13).fontWeight(FontWeight.Medium).fontColor(Theme.ink)
            TextArea({ placeholder: 'session=MTc4MzQyOTkyN3xE...', text: this.cookieInput })
              .height(80).fontSize(12).fontFamily(Theme.fontMono)
              .backgroundColor(Theme.surfaceRaised)
              .onChange((value: string) => { this.cookieInput = value; })
            Text('从桌面浏览器 F12 → Application → Cookies 复制 session 值')
              .fontSize(11).fontColor(Theme.muted)
            TextInput({ placeholder: 'API Key（可选）', text: this.apiKeyInput })
              .height(44).fontSize(12).fontFamily(Theme.fontMono)
              .backgroundColor(Theme.surfaceRaised)
              .onChange((value: string) => { this.apiKeyInput = value; })
            Button('保存并验证')
              .width('100%').height(44)
              .backgroundColor(Theme.accent).fontColor('#f7faf7')
              .borderRadius(4).enabled(!this.busy)
              .onClick(() => this.saveManual())
          }
          .width('100%')
          .padding(16)
          .border({ width: 1, color: Theme.line, radius: 6 })

          if (this.statusText.length > 0) {
            Text(this.statusText)
              .fontSize(12).fontColor(this.statusIsError ? Theme.error : Theme.accent)
              .backgroundColor(this.statusIsError ? Theme.errorBg : Theme.accentSoft)
              .width('100%').padding(10).borderRadius(2)
          }
        }
        .width('100%')
        .constraintSize({ maxWidth: Theme.contentMaxWidth(this.bp) })
        .padding({ left: Theme.pagePadding(this.bp), right: Theme.pagePadding(this.bp), top: 48, bottom: 48 })
      }
      .width('100%').height('100%')
      .scrollBar(BarState.Auto)
      .align(Alignment.Top)
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
```

- [ ] **Step 3: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。

- [ ] **Step 4: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main/ets/pages/ConfigPage.ets harmony/entry/src/main/ets/pages/LoginPage.ets
git commit -m "feat: 配置页与 WebView 登录页"
```

---

### Task 8: 看板页、入口 Ability 与 module 配置

**Files:**
- Create: `harmony/entry/src/main/ets/pages/DashboardPage.ets`
- Modify: `harmony/entry/src/main/ets/entryability/EntryAbility.ets`（initStore + 断点注册 + 初始路由）
- Modify: `harmony/entry/src/main/module.json5`（INTERNET 权限、deviceTypes）
- Modify: `harmony/entry/src/main/resources/base/profile/main_pages.json`（三页面路由表）
- Delete: 模板默认页 `entry/src/main/ets/pages/Index.ets`（若模板存在）

**Interfaces:**
- Produces: 完整可运行的三页面应用（待 Task 9 签名部署）。
- Consumes: Task 2-7 全部。

- [ ] **Step 1: DashboardPage.ets**

```typescript
import { router } from '@kit.ArkUI';
import { Theme } from '../common/Theme';
import { GridBackground } from '../common/GridBackground';
import { formatTokens, formatYuan, ApiError } from '../common/Format';
import { getCookie, saveSnapshot } from '../model/Store';
import { fetchDashboard, DashboardData } from '../model/Api';

const AUTO_REFRESH_INTERVAL_MS = 60_000; // onPageShow 自动刷新节流

// 本地时区 RFC3339：动态计算偏移，任意时区正确（与桌面版 Local::now().to_rfc3339() 同口径）
function localRfc3339(date: Date): string {
  const offsetMin = -date.getTimezoneOffset();
  const sign = offsetMin >= 0 ? '+' : '-';
  const abs = Math.abs(offsetMin);
  const hh = String(Math.floor(abs / 60)).padStart(2, '0');
  const mm = String(abs % 60).padStart(2, '0');
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${sign}${hh}:${mm}`;
}

@Entry
@Component
struct DashboardPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State data: DashboardData | null = null;
  @State errorMsg: string = '';
  @State refreshing: boolean = false;
  @State lastUpdatedText: string = '等待同步';
  private lastRefreshAt: number = 0;

  async onPageShow(): Promise<void> {
    const now = Date.now();
    if (now - this.lastRefreshAt >= AUTO_REFRESH_INTERVAL_MS) {
      await this.refresh();
    }
  }

  async refresh(): Promise<void> {
    this.refreshing = true;
    this.errorMsg = '';
    try {
      const cookie = (await getCookie()) ?? '';
      const data = await fetchDashboard(cookie);
      this.data = data;
      this.lastRefreshAt = Date.now();
      this.lastUpdatedText = '刚刚更新';
      await saveSnapshot({
        savedAt: localRfc3339(new Date()),
        todayYuan: data.todayYuan,
        totalTokens: data.todayTokens,
        remaining: data.remaining,
        used: data.used,
        total: data.total,
        requestCount: data.requestCount,
      });
    } catch (error) {
      if (error instanceof ApiError && error.isAuthError) {
        router.clear();
        router.replaceUrl({ url: 'pages/ConfigPage' });
        return;
      }
      // 其他错误：保留已有数据，显示重试信息
      this.errorMsg = error instanceof ApiError
        ? error.message + '（点击刷新重试）'
        : '刷新失败，请重试';
    } finally {
      this.refreshing = false;
    }
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      GridBackground()
      Refresh({ refreshing: $$this.refreshing }) {
        Scroll() {
          Column({ space: 0 }) {
            // 顶栏
            Row() {
              Text('AMAX / DAILY')
                .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                .fontColor(Theme.accent).letterSpacing(2)
              Blank()
              Text(this.lastUpdatedText).fontSize(10).fontColor(Theme.muted)
              Text('⟳').fontSize(18).fontColor(Theme.muted).margin({ left: 12 })
                .onClick(() => this.refresh())
              Text('⚙').fontSize(16).fontColor(Theme.muted).margin({ left: 12 })
                .onClick(() => router.pushUrl({ url: 'pages/ConfigPage' }))
            }
            .width('100%').padding({ top: 14, bottom: 14 })

            if (this.errorMsg.length > 0) {
              Text(this.errorMsg)
                .fontSize(12).fontColor(Theme.error).backgroundColor(Theme.errorBg)
                .width('100%').padding(10).margin({ bottom: 14 })
            }

            if (this.data !== null) {
              // 账户头
              Text(this.data.displayName).fontSize(20).fontWeight(FontWeight.Bold)
                .fontColor(Theme.ink)
              Text(`${this.data.requestCount} 次请求`)
                .fontFamily(Theme.fontMono).fontSize(10).fontColor(Theme.muted)
                .margin({ top: 6 })

              // COST TODAY
              Column({ space: 8 }) {
                Text('COST TODAY')
                  .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                  .fontColor(Theme.muted).letterSpacing(2)
                Text(formatYuan(this.data.todayYuan))
                  .fontFamily(Theme.fontMono)
                  .fontSize(this.bp === 'sm' ? 40 : 46).fontWeight(600)
                  .fontColor(Theme.ink)
                Text(this.data.logsAvailable ? '' : '日志暂不可用，仅显示额度')
                  .fontSize(11).fontColor(Theme.muted)
              }
              .alignItems(HorizontalAlign.Start)
              .width('100%').padding({ top: 36, bottom: 28 })

              // TOKENS 网格
              Row() {
                Column({ space: 6 }) {
                  Text('TOKENS')
                    .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                    .fontColor(Theme.muted).letterSpacing(2)
                  Text(formatTokens(this.data.todayTokens))
                    .fontFamily(Theme.fontMono).fontSize(18).fontWeight(600).fontColor(Theme.ink)
                }
                .alignItems(HorizontalAlign.Start).layoutWeight(0.9)

                Column({ space: 6 }) {
                  Text('INPUT / OUTPUT')
                    .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                    .fontColor(Theme.muted).letterSpacing(2)
                  Text(`${formatTokens(this.data.todayInput)} / ${formatTokens(this.data.todayOutput)}`)
                    .fontFamily(Theme.fontMono).fontSize(18).fontWeight(600).fontColor(Theme.ink)
                }
                .alignItems(HorizontalAlign.Start).layoutWeight(1.35)
                .border({ width: { left: 1 }, color: Theme.line })
                .padding({ left: 18 })
              }
              .width('100%')
              .padding({ top: 16, bottom: 16 })
              .border({ width: { top: 1, bottom: 1 }, color: Theme.line })

              // 额度块
              Column({ space: 10 }) {
                Row() {
                  Column({ space: 4 }) {
                    Text('AVAILABLE QUOTA')
                      .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                      .fontColor(Theme.muted).letterSpacing(2)
                    Text(`${this.data.percent.toFixed(1)}%`)
                      .fontFamily(Theme.fontMono).fontSize(28).fontWeight(600).fontColor(Theme.ink)
                  }
                  .alignItems(HorizontalAlign.Start)
                  Blank()
                  Text(`剩余 ${formatYuan(this.data.remaining)}`)
                    .fontFamily(Theme.fontMono).fontSize(11).fontColor(Theme.accent)
                }
                .width('100%')

                Progress({ value: this.data.percent, total: 100, type: ProgressType.Linear })
                  .width('100%').color(Theme.accent)

                Row() {
                  Text(`USED ${formatYuan(this.data.used)}`).fontSize(9).fontFamily(Theme.fontMono)
                    .fontColor(Theme.muted)
                  Blank()
                  Text(`TOTAL ${formatYuan(this.data.total)}`).fontSize(9)
                    .fontFamily(Theme.fontMono).fontColor(Theme.muted)
                }
                .width('100%')
              }
              .width('100%')
              .padding({ left: 16, top: 4, bottom: 4 })
              .border({ width: { left: 2 }, color: Theme.accent })
              .margin({ top: 30 })
            } else {
              Text('正在加载…').fontSize(13).fontColor(Theme.muted).margin({ top: 60 })
            }
          }
          .width('100%')
          .constraintSize({ maxWidth: Theme.contentMaxWidth(this.bp) })
          .padding({ left: Theme.pagePadding(this.bp), right: Theme.pagePadding(this.bp), bottom: 40 })
        }
        .width('100%').height('100%')
        .scrollBar(BarState.Auto)
        .align(Alignment.Top)
      }
      .width('100%').height('100%')
      .onRefreshing(() => this.refresh())
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
```

注意：`savedAt` 经 `localRfc3339()` 动态构造本地时区偏移，禁止硬编码时区串。

- [ ] **Step 2: EntryAbility.ets（在模板基础上修改）**

模板已生成 `onCreate` / `onWindowStageCreate`，在其中加入：

```typescript
import { initStore, getCookie } from '../model/Store';
import { BreakpointSystem } from '../common/BreakpointSystem';

// 类内新增成员
private breakpointSystem: BreakpointSystem = new BreakpointSystem();

// onCreate 内（super 之后）：
await initStore(this.context);

// onWindowStageCreate 内 loadContent 改为按凭据决定初始页：
const mainWindow = await windowStage.getMainWindow();
this.breakpointSystem.register(mainWindow.getUIContext());
const hasCookie = (await getCookie()) !== null;
windowStage.loadContent(hasCookie ? 'pages/DashboardPage' : 'pages/ConfigPage', (err) => {
  if (err.code) {
    console.error(`加载初始页失败：${JSON.stringify(err)}`);
  }
});

// onDestroy 内：
this.breakpointSystem.unregister();
```

（保留模板的日志与其他逻辑；`import window from '@ohos.window'` 按模板既有引入。）

- [ ] **Step 3: module.json5 权限与设备类型**

在 `module` 节点内：
- `"deviceTypes"` 改为 `["phone", "tablet", "2in1"]`
- 新增请求权限：

```json5
"requestPermissions": [
  {
    "name": "ohos.permission.INTERNET"
  }
]
```

- [ ] **Step 4: main_pages.json 路由表**

`entry/src/main/resources/base/profile/main_pages.json` 的 `src` 数组改为：

```json
{
  "src": [
    "pages/DashboardPage",
    "pages/ConfigPage",
    "pages/LoginPage"
  ]
}
```

删除模板默认页 `entry/src/main/ets/pages/Index.ets`（及模板对其的引用）。

- [ ] **Step 5: 编译验证**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli build`
Expected: BUILD SUCCESSFUL。

- [ ] **Step 6: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/entry/src/main
git commit -m "feat: 看板页与入口路由配置"
```

---

### Task 9: 签名、部署与 GUI 验证

**Files:**
- Modify: `harmony/build-profile.json5`（devecocli signature generate 自动写入签名配置）

**Interfaces:**
- Consumes: Task 1-8 全部。
- Produces: 真机/模拟器可运行的已签名应用；GUI 验证通过。

**用户协作步骤（需人工执行/确认）:**
1. `devecocli auth login` —— 浏览器 OAuth 登录华为开发者账号（控制器不可代操作，等待用户确认完成）
2. 连接真机（hdc，开启开发者模式 + USB 调试）或启动模拟器：
   - 模拟器：`/c/nvm4w/nodejs/devecocli emulator list` 查看实例；无实例时 `emulator create <name> --device-type phone --os-version <版本>` 需用户在 DevEco Studio Device Manager 中完成或接受协议（`emulator license accept`）；镜像下载约 30 分钟
3. `devecocli signature generate --product default`（在 harmony/ 目录执行，需已登录 + 已连接设备）

- [ ] **Step 1: 确认登录态**

Run: `/c/nvm4w/nodejs/devecocli auth status`
Expected: 显示已登录用户。未登录则请用户执行 `devecocli auth login`。

- [ ] **Step 2: 确认设备**

Run: `/c/nvm4w/nodejs/devecocli device list`
Expected: 至少一个真机或运行中的模拟器。无设备则按上述用户协作步骤 2 处理。

- [ ] **Step 3: 生成签名**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli signature generate --product default`
Expected: `~/.ohos/config/` 生成 p12/csr/cer/p7b；`build-profile.json5` 写入 signingConfigs。
失败码处置：401 重新登录；`205389938` provision 超限需用户在 AGC 控制台清理旧测试 profile。

- [ ] **Step 4: 部署运行**

Run: `cd D:/file/rs-project/amax/harmony && /c/nvm4w/nodejs/devecocli run`
Expected: 构建 → 安装 → 启动，设备显示应用界面。
签名冲突（`install sign info inconsistent`）时：`devecocli run --uninstall`。

- [ ] **Step 5: 仪器测试（ohosTest，验证 HUKS 与 RDB 真实行为）**

创建 `entry/src/ohosTest/ets/test/SecretStore.test.ets`（模板未生成 ohosTest 目录时一并创建，并在 `ohosTest/ets/testrunner/` 的测试注册入口注册本用例；context 获取方式经 `devecocli docs search "ohosTest abilityDelegator context"` 查证）：

```typescript
import { describe, it, expect } from '@ohos/hypium';
import { encryptSecret, decryptSecret } from '../../../main/ets/model/Secret';
import { initStore, saveCookie, getCookie, clearCredentials, saveSnapshot } from '../../../main/ets/model/Store';

export default function secretStoreTest() {
  describe('secretStoreInstrumentTest', () => {
    it('encryptDecryptRoundTrip', 0, async (done: Function) => {
      const plain = 'session=test-cookie-value';
      const encrypted = await encryptSecret(plain);
      expect(encrypted.startsWith('huks:v1:')).assertTrue();
      const decrypted = await decryptSecret(encrypted);
      expect(decrypted).assertEqual(plain);
      done();
    });
    it('legacyPlaintextPassthrough', 0, async (done: Function) => {
      const legacy = 'session=plaintext';
      expect(await decryptSecret(legacy)).assertEqual(legacy);
      done();
    });
    it('cookieStoreRoundTrip', 0, async (done: Function) => {
      await initStore(getContext());
      await saveCookie('session=abc; other=1', 'manual');
      expect(await getCookie()).assertEqual('session=abc; other=1');
      await clearCredentials();
      expect(await getCookie()).assertNull();
      done();
    });
    it('snapshotInsert', 0, async (done: Function) => {
      await saveSnapshot({
        savedAt: '2026-08-02T10:00:00+08:00',
        todayYuan: 1.5, totalTokens: 1000, remaining: 90, used: 10, total: 100, requestCount: 5,
      });
      done();
    });
  });
}
```

Run（需已连接设备）:
```bash
cd D:/file/rs-project/amax/harmony
hvigorw test -p module=entry -p product=default -p testType=InstrumentTest --no-daemon
```
Expected: 4 个用例 PASS。命令形式不兼容时经 `devecocli docs read "开发指南/开发自测试/测试框架/代码测试/Instrument_Test/ide-instrument-test"` 查证。

- [ ] **Step 6: GUI 验证清单**

用 `/c/nvm4w/nodejs/devecocli ui screenshot --path <仓库>/tmp/harmony-gui-<N>.png` 截图辅助核验（截图属临时文件，放 tmp/）：

- [ ] 无凭据启动 → 配置页（AMAX / ACCESS + 两个登录入口）
- [ ] 前往登录 → WebView 加载官网登录页
- [ ] 登录成功 → 自动跳看板，显示真实账户数据（用户名 / 费用 / Token / 额度）
- [ ] 下拉刷新生效，顶栏刷新按钮生效
- [ ] 设置入口回配置页；"重新登录"清除凭据后回登录流程
- [ ] 手动粘贴路径：粘贴 Cookie → 保存并验证 → 看板
- [ ] 认证错误路径：清除 Cookie 后刷新 → 回配置页（可篡改凭据模拟）
- [ ] 日志失败容错：断网后刷新 → 错误横幅 + 保留已有数据
- [ ] 断点自适应：模拟器切换折叠展开/平板尺寸（或 `emulator fold`/旋转）→ 内容区限宽居中、边距递增
- [ ] 快照写入：`devecocli log --bundle-name com.amax.dashboard --keyword 快照 --from 10m` 无报错（或 hdc 查库，可选）

- [ ] **Step 7: 缺陷修复（如有）**

按缺陷归属任务范围修复，独立提交 `fix: <描述>`；每次修复后 `devecocli run` 复验。

- [ ] **Step 8: 提交签名配置变更**

`build-profile.json5` 的 signingConfigs 含加密密码（AES-128-GCM，devecocli 写入），可提交（与 DevEco Studio 工程实践一致）：

```bash
cd D:/file/rs-project/amax
git add harmony/build-profile.json5
git commit -m "chore: 添加调试签名配置"
```

---

### Task 10: 文档同步

**Files:**
- Create: `harmony/README.md`
- Modify: 根目录 `CLAUDE.md`

- [ ] **Step 1: harmony/README.md**

```markdown
# AMAX Dashboard（HarmonyOS 版）

AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型）。子项目一期：配置页（应用内 WebView 登录 + 手动粘贴）、看板页、HUKS 凭据加密、断点自适应（phone / tablet / 2in1）。

## 开发命令

```bash
# 构建
/c/nvm4w/nodejs/devecocli build

# 本地单元测试（entry/src/test，@ohos/hypium，无需设备）
hvigorw test -p module=entry -p product=default -p testType=LocalTest --no-daemon

# 签名（首次：先 devecocli auth login，连接设备后执行）
/c/nvm4w/nodejs/devecocli signature generate --product default

# 部署运行
/c/nvm4w/nodejs/devecocli run

# 截图 / 日志
/c/nvm4w/nodejs/devecocli ui screenshot --path ./tmp/shot.png
/c/nvm4w/nodejs/devecocli log --bundle-name com.amax.dashboard --from 10m

# 本地官方文档检索
/c/nvm4w/nodejs/devecocli docs search "<关键词>" --limit 5
/c/nvm4w/nodejs/devecocli docs read "<documentId>"
```

## 结构

- `entry/src/main/ets/pages/` — DashboardPage / ConfigPage / LoginPage
- `entry/src/main/ets/model/` — Api（官网接口）/ Store（preferences + RDB）/ Secret（HUKS）
- `entry/src/main/ets/common/` — Theme / BreakpointSystem / GridBackground / Format

## 路线

- 二期：统计页（趋势 / 模型分布 / 汇总 / 余额）、数据导出、低余额通知、后台定时刷新
```

- [ ] **Step 2: 根 CLAUDE.md 追加鸿蒙段落**

在 `## 数据与界面流程` 段落之后新增：

```markdown
## HarmonyOS 版

`harmony/` 为 AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型），与桌面版共享数据口径与官网接口约定：

- 页面：DashboardPage / ConfigPage（应用内 WebView 登录 + 手动粘贴）/ LoginPage（官网登录页，`onLoadIntercept` 捕获重定向，`WebCookieManager.fetchCookieSync` 提取 Cookie）。
- model 层与桌面版模块对应：`Api.ets`（同 api.rs 协议与容错）、`Store.ets`（同 db.rs 快照 schema，含 request_count）、`Secret.ets`（HUKS AES-256-GCM 替代 DPAPI，`huks:v1:` 前缀 + 旧明文兼容）。
- 断点自适应：`BreakpointSystem`（sm<600 / 600≤md<840 / lg≥840 vp）+ 内容区限宽居中；deviceTypes 为 phone / tablet / 2in1。
- 工具链：devecocli（build / run / signature / docs / ui / log）；本地单元测试 `hvigorw test -p testType=LocalTest`（@ohos/hypium）；API 细节以 `devecocli docs` 查证为准。
- 开发命令详见 `harmony/README.md`。
```

- [ ] **Step 3: 提交**

```bash
cd D:/file/rs-project/amax
git add harmony/README.md CLAUDE.md
git commit -m "docs: 同步鸿蒙版开发文档"
```

---

## 交付定义

- 10 个任务全部完成，`devecocli build` 通过，本地单元测试全绿。
- 真机/模拟器 GUI 验证清单全部勾选。
- 每个任务独立提交（中文祈使句），共约 10-12 个提交。
- harmony/README.md 与根 CLAUDE.md 同步。
