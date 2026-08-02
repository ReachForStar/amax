// H4/H5 运行期验证：HUKS 加解密往返、旧明文兼容、Cookie 存取、快照写入
// 仪器测试跑在真实 HarmonyOS 运行时（模拟器/真机），覆盖单元测试无法触达的
// HUKS 密钥库与 preferences/RDB 系统 API
// 上下文来源：经 TestAbility 透传的 ability context（getAppContext 在测试进程对数据 API 无效，报 401）
// 密钥重置：删除可能由历史版本/前序任务残留的同名密钥，确保以当前参数集重新生成，
// 否则旧密钥参数与会话不匹配会报 "The parameter does not exist"（生产首次使用自行生成，无此问题）
import { describe, it, expect, beforeAll } from '@ohos/hypium';
import { huks } from '@kit.UniversalKeystoreKit';
import { encryptSecret, decryptSecret } from '../../../main/ets/model/Secret';
import { initStore, saveCookie, getCookie, saveSnapshot, SnapshotRow } from '../../../main/ets/model/Store';
import { getTestContext } from '../testability/TestContext';

const SECRET_KEY_ALIAS = 'amax_secret_key';

export default function secretStoreTest() {
  describe('secretStoreTest', () => {
    beforeAll(async () => {
      await initStore(getTestContext());
      try {
        await huks.deleteKeyItem(SECRET_KEY_ALIAS, { properties: [] });
      } catch (e) {
        // 密钥不存在时删除抛错，忽略
      }
    });

    it('huks_roundtrip_restores_plaintext', 0, async () => {
      const plain = 'session=MTc4MzQy; Path=/; HttpOnly';
      const stored = await encryptSecret(plain);
      expect(stored.startsWith('huks:v1:')).assertTrue();
      const decrypted = await decryptSecret(stored);
      expect(decrypted).assertEqual(plain);
    });

    it('decrypt_legacy_plaintext_passthrough', 0, async () => {
      const decrypted = await decryptSecret('legacy-plain-cookie');
      expect(decrypted).assertEqual('legacy-plain-cookie');
    });

    it('cookie_save_and_get_roundtrip', 0, async () => {
      await saveCookie('session=itest9527', 'manual');
      const got = await getCookie();
      expect(got).assertEqual('session=itest9527');
    });

    it('snapshot_insert_succeeds', 0, async () => {
      const row: SnapshotRow = {
        savedAt: '2026-08-03T10:00:00+08:00',
        todayYuan: 1.5,
        totalTokens: 12345,
        remaining: 90.25,
        used: 9.75,
        total: 100,
        requestCount: 42
      };
      // saveSnapshot 内部 insert 失败（异常或负 rowId）会 reject，不抛错即写入成功
      await saveSnapshot(row);
    });
  });
}
