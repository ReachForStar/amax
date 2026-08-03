// Canvas 环图：按 segments 数值比例绘制扇环，中心显示汇总文本
// 纯展示组件：数据全部经 props 传入；父组件数据变化后递增 version 触发重绘
import { Theme } from './Theme';

// 环图分段定义（ArkTS 禁止对象字面量类型，显式导出供父组件构造；label 供父组件画图例）
export interface DonutSegment {
  label: string;
  value: number;
  color: string;
}

const RING_PADDING = 8;   // 外半径与画布边缘留白
const INNER_RATIO = 0.62; // 内半径 = 外半径 × 0.62

@Component
export struct DonutChart {
  @Prop segments: DonutSegment[] = [];
  @Prop centerText: string = '';
  @Prop @Watch('onVersionChange') version: number = 0;
  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);

  onVersionChange(): void {
    this.draw();
  }

  build() {
    Canvas(this.ctx)
      .width('100%')
      .height('100%')
      .onReady(() => this.draw())
  }

  private draw(): void {
    const w = this.ctx.width;
    const h = this.ctx.height;
    if (w <= 0 || h <= 0) {
      return; // 尺寸未就绪（onReady 前），由 onReady 补绘
    }
    this.ctx.clearRect(0, 0, w, h);
    const cx = w / 2;
    const cy = h / 2;
    const outer = Math.min(w, h) / 2 - RING_PADDING;
    if (outer <= 0) {
      return; // 画布小于留白尺寸，无可绘制内容
    }
    const inner = outer * INNER_RATIO;

    // 有效总和：仅统计有限正数，负数/NaN 不参与占比
    let total = 0;
    for (const seg of this.segments) {
      const value = seg.value;
      if (Number.isFinite(value) && value > 0) {
        total += value;
      }
    }

    if (total <= 0) {
      // 总和为 0：整环 line 色 + 中心"暂无数据"
      this.drawRingSegment(cx, cy, outer, inner, 0, Math.PI * 2, Theme.line);
      this.drawCenterText('暂无数据', cx, cy, Theme.muted, 12);
      return;
    }

    // 12 点方向起始、顺时针按值占比逐段绘制；零值/非法段跳过（避免零长度弧）
    let start = -Math.PI / 2;
    for (const seg of this.segments) {
      const value = seg.value;
      if (!Number.isFinite(value) || value <= 0) {
        continue;
      }
      const sweep = value / total * Math.PI * 2;
      this.drawRingSegment(cx, cy, outer, inner, start, start + sweep, seg.color);
      start += sweep;
    }

    if (this.centerText.length > 0) {
      this.drawCenterText(this.centerText, cx, cy, Theme.ink, 14);
    }
  }

  // 扇环路径：外弧顺时针 + 内弧逆时针回绕，闭合后填充
  private drawRingSegment(cx: number, cy: number, outer: number, inner: number,
    start: number, end: number, color: string): void {
    this.ctx.fillStyle = color;
    this.ctx.beginPath();
    this.ctx.arc(cx, cy, outer, start, end, false);
    this.ctx.arc(cx, cy, inner, end, start, true);
    this.ctx.closePath();
    this.ctx.fill();
  }

  private drawCenterText(text: string, cx: number, cy: number, color: string, size: number): void {
    this.ctx.font = `${size}px monospace`;
    this.ctx.fillStyle = color;
    this.ctx.textAlign = 'center';
    this.ctx.textBaseline = 'middle';
    this.ctx.fillText(text, cx, cy);
  }
}
