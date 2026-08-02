// 测试上下文桥：将 TestAbility 的 UIAbilityContext 透传给测试用例
// 原因：仪器测试进程内 abilityDelegator.getAppContext() 返回的应用级 context 对
// preferences/RDB 无效（报 401 context invalid），而 ability 的 this.context 有效
import { Context } from '@kit.AbilityKit';

let holder: Context | null = null;

export function setTestContext(context: Context): void {
  holder = context;
}

export function getTestContext(): Context {
  if (holder === null) {
    throw new Error('test context not initialized: TestAbility.onCreate 未执行');
  }
  return holder;
}
