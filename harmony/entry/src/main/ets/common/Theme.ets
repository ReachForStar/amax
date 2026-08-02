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
