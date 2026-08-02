// 仪器测试运行器：拉起 TestAbility，由其在 ability 生命周期内驱动 hypium
// （纯 runner 进程上下文对 preferences/RDB 无效，需经 ability 初始化数据令牌）
import { abilityDelegatorRegistry, TestRunner } from '@kit.TestKit';
import { UIAbility, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';

const DOMAIN: number = 0x0000;
const TAG: string = 'testTag';
const TEST_ABILITY_NAME: string = 'TestAbility';
const TEST_MODULE_NAME: string = 'entry_test';

let abilityDelegator: abilityDelegatorRegistry.AbilityDelegator;
let abilityDelegatorArguments: abilityDelegatorRegistry.AbilityDelegatorArgs;

async function onAbilityCreateCallback(data: UIAbility): Promise<void> {
  hilog.info(DOMAIN, TAG, 'onAbilityCreateCallback, data: %{public}s', JSON.stringify(data));
}

async function addAbilityMonitorCallback(err: BusinessError): Promise<void> {
  if (err) {
    hilog.error(DOMAIN, TAG, 'addAbilityMonitorCallback: %{public}s', JSON.stringify(err) ?? '');
  }
}

export default class OpenHarmonyTestRunner implements TestRunner {
  constructor() {
  }

  onPrepare(): void {
    hilog.info(DOMAIN, TAG, '%{public}s', 'OpenHarmonyTestRunner OnPrepare');
  }

  async onRun(): Promise<void> {
    hilog.info(DOMAIN, TAG, '%{public}s', 'OpenHarmonyTestRunner onRun');
    abilityDelegatorArguments = abilityDelegatorRegistry.getArguments();
    abilityDelegator = abilityDelegatorRegistry.getAbilityDelegator();
    const abilityMonitor: abilityDelegatorRegistry.AbilityMonitor = {
      abilityName: TEST_ABILITY_NAME,
      onAbilityCreate: onAbilityCreateCallback,
    };
    abilityDelegator.addAbilityMonitor(abilityMonitor, addAbilityMonitorCallback);
    // aa test 的 -m 不会透传到 runner parameters，moduleName 缺省回退测试模块名 entry_test
    const moduleName: string = abilityDelegatorArguments.parameters['-m'] ?? TEST_MODULE_NAME;
    const want: Want = {
      bundleName: abilityDelegatorArguments.bundleName,
      abilityName: TEST_ABILITY_NAME,
      moduleName: moduleName
    };
    await abilityDelegator.startAbility(want);
    hilog.info(DOMAIN, TAG, '%{public}s', 'OpenHarmonyTestRunner onRun end');
  }
}
