// 测试承载 Ability：在 ability 生命周期内驱动 hypium，使 getAppContext 返回的 context 数据令牌有效
// （纯 runner 进程无 activity 上下文时，preferences/RDB 取 getAppContext 会报 401 context invalid）
// hypium 1.0.25 签名：hypiumTest(delegator, arguments, testsuite)；delegator 经全局注册表获取
import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';
import { abilityDelegatorRegistry } from '@kit.TestKit';
import { Hypium } from '@ohos/hypium';
import testsuite from '../test/List.test';
import { setTestContext } from './TestContext';

export default class TestAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    console.info('TestAbility onCreate');
    // 先透传有效的 ability context，再驱动 hypium（beforeAll 经 getTestContext 取用）
    setTestContext(this.context);
    const delegator = abilityDelegatorRegistry.getAbilityDelegator();
    const args = abilityDelegatorRegistry.getArguments();
    Hypium.hypiumTest(delegator, args, testsuite);
  }

  onDestroy(): void {
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('testability/pages/Index');
  }

  onWindowStageDestroy(): void {
  }

  onForeground(): void {
  }

  onBackground(): void {
  }
}
