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
