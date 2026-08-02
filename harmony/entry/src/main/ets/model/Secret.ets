// HUKS 凭据加密：AES-256-GCM，密钥由系统通用密钥库托管，密钥材料不出 TEE
// 存储格式 huks:v1:<base64(iv | 密文 | tag)>；无该前缀的旧明文原样返回（兼容迁移）
import { huks } from '@kit.UniversalKeystoreKit';
import { cryptoFramework } from '@kit.CryptoArchitectureKit';
import { util } from '@kit.ArkTS';

const KEY_ALIAS = 'amax_secret_key';
const PREFIX = 'huks:v1:';
const IV_LENGTH = 12;
// GCM 认证标签长度（HUKS GCM 默认 128 位），仅用于解密输入的下限校验
const GCM_TAG_LENGTH = 16;

// purpose 枚举按位或后的类型为 number，形参声明为 number，
// 规避 ArkTS 严格模式下 number 与枚举类型的可赋值性风险
function baseProperties(purpose: number, nonce?: Uint8Array): Array<huks.HuksParam> {
  const props: Array<huks.HuksParam> = [
    { tag: huks.HuksTag.HUKS_TAG_ALGORITHM, value: huks.HuksKeyAlg.HUKS_ALG_AES },
    { tag: huks.HuksTag.HUKS_TAG_KEY_SIZE, value: huks.HuksKeySize.HUKS_AES_KEY_SIZE_256 },
    { tag: huks.HuksTag.HUKS_TAG_PURPOSE, value: purpose },
    { tag: huks.HuksTag.HUKS_TAG_PADDING, value: huks.HuksKeyPadding.HUKS_PADDING_NONE },
    { tag: huks.HuksTag.HUKS_TAG_BLOCK_MODE, value: huks.HuksCipherMode.HUKS_MODE_GCM },
  ];
  if (nonce !== undefined) {
    // GCM 强制要求携带 NONCE 与 AAD 两个 TAG（AAD 可为空字节数组，TAG 不能省略）
    props.push({ tag: huks.HuksTag.HUKS_TAG_NONCE, value: nonce });
    props.push({ tag: huks.HuksTag.HUKS_TAG_ASSOCIATED_DATA, value: new Uint8Array(0) });
  }
  return props;
}

const ENCRYPT_DECRYPT: number =
  huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_ENCRYPT | huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_DECRYPT;

// 串行化懒生成：防止并发调用方同时判定"不存在"而重复生成。
// generateKeyItem 对已存在别名不报错（错误码表无同名项），覆盖密钥会使存量密文永久不可解
let ensureKeyTask: Promise<void> | null = null;

function ensureKey(): Promise<void> {
  if (ensureKeyTask === null) {
    ensureKeyTask = generateKeyIfAbsent().catch((error: Error): Promise<void> => {
      // 失败时清掉缓存允许下次重试；成功结果永久复用
      ensureKeyTask = null;
      return Promise.reject(error);
    });
  }
  return ensureKeyTask;
}

async function generateKeyIfAbsent(): Promise<void> {
  // 用 hasKeyItem（11+）：密钥不存在时返回 false；
  // isKeyItemExist（9+）在密钥不存在时抛错误码 12000011，取反判断不可用
  const exists = await huks.hasKeyItem(KEY_ALIAS, { properties: [] });
  if (!exists) {
    await huks.generateKeyItem(KEY_ALIAS, { properties: baseProperties(ENCRYPT_DECRYPT) });
  }
}

// generateRandomSync 返回 DataBlob，唯一字段为 data（无 randomNumber 字段）
function randomBytes(length: number): Uint8Array {
  return cryptoFramework.createRandom().generateRandomSync(length).data;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

// HuksReturnResult.outData 为可选字段，缺失时回退空数组，保证拼接与后续解码安全
function outDataOf(result: huks.HuksReturnResult): Uint8Array {
  return result.outData ?? new Uint8Array(0);
}

// 三段式 GCM 会话：init（inData 置空）→ update（携带数据）→ finish（inData 置空）
// 任一阶段出错先按文档要求调用 abortSession 释放 handle，再原样抛出错误（不吞错）
async function runSession(purpose: number, nonce: Uint8Array, inData: Uint8Array): Promise<Uint8Array> {
  const options: huks.HuksOptions = {
    properties: baseProperties(purpose, nonce),
    inData: new Uint8Array(0),
  };
  const session = await huks.initSession(KEY_ALIAS, options);
  try {
    options.inData = inData;
    const update = await huks.updateSession(session.handle, options);
    options.inData = new Uint8Array(0);
    const finish = await huks.finishSession(session.handle, options);
    // 加密：update 返回密文、finish 返回 tag；解密：明文来自 update，finish 校验 tag 输出通常为空
    return concat(outDataOf(update), outDataOf(finish));
  } catch (error) {
    try {
      await huks.abortSession(session.handle, options);
    } catch (abortError) {
      // abort 失败只影响资源回收，记录日志；业务原始错误仍在下方重抛
      console.error(`Secret: abortSession failed: ${JSON.stringify(abortError)}`);
    }
    throw error as Error;
  }
}

export async function encryptSecret(plain: string): Promise<string> {
  await ensureKey();
  const iv = randomBytes(IV_LENGTH);
  const plainBytes = new util.TextEncoder().encodeInto(plain);
  const cipher = await runSession(huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_ENCRYPT, iv, plainBytes);
  // base64(iv(12B) | 密文 | tag)
  return PREFIX + new util.Base64Helper().encodeToStringSync(concat(iv, cipher));
}

export async function decryptSecret(stored: string): Promise<string> {
  // 旧明文兼容：无 huks:v1: 前缀原样返回（迁移路径）
  if (!stored.startsWith(PREFIX)) {
    return stored;
  }
  await ensureKey();
  const blob = new util.Base64Helper().decodeSync(stored.slice(PREFIX.length));
  // 合法 blob 至少包含 iv + tag，短于此为损坏记录，提前拒绝避免无效 HUKS 会话
  if (blob.length < IV_LENGTH + GCM_TAG_LENGTH) {
    throw new Error('invalid huks secret: blob shorter than iv + gcm tag');
  }
  const iv = blob.slice(0, IV_LENGTH);
  const cipher = blob.slice(IV_LENGTH);
  const out = await runSession(huks.HuksKeyPurpose.HUKS_KEY_PURPOSE_DECRYPT, iv, cipher);
  // 解密失败（篡改/密钥异常）由 HUKS 抛错，调用方视为无效凭据
  return util.TextDecoder.create('utf-8').decodeToString(out);
}
