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
