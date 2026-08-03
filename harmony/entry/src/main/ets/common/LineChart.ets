// Canvas 折线图：多序列、双轴（左 ¥ / 右 Tokens）、null 断开、触摸十字提示
// 纯展示组件：数据全部经 props 传入；父组件数据变化后递增 version 触发重绘
// 坐标约定：Canvas onReady/触摸坐标均为 vp，context.width/height 也是 vp，
// 因此直接按组件自身坐标系绘制与命中换算，无需像素密度转换
import { Theme } from './Theme';
import { formatTokens } from './Format';

// 折线序列定义（ArkTS 禁止对象字面量类型，显式导出供父组件构造）
export interface ChartSeries {
  name: string;
  color: string;
  data: Array<number | null>;
  dashed?: boolean;
}

// 画布几何（边距换算结果，绘制与触摸命中共用）
interface ChartLayout {
  w: number;
  h: number;
  plotX: number;
  plotY: number;
  plotW: number;
  plotH: number;
}

// 浮层单行文本：日期行用 ink，序列行用各自序列色
interface TooltipLine {
  text: string;
  color: string;
}

const MARGIN_LEFT = 48;          // 左轴标签区
const MARGIN_RIGHT_DUAL = 48;    // 双轴时右轴标签区
const MARGIN_RIGHT_SINGLE = 12;  // 单轴右侧小留白，防止端点圆被裁剪
const MARGIN_TOP = 12;
const MARGIN_BOTTOM = 24;        // x 标签区
const Y_DIVISIONS = 4;           // y 轴 4 格刻度
const MAX_X_LABELS = 8;          // x 标签抽稀上限
const POINT_RADIUS = 2;          // 数据点半径
const AXIS_FALLBACK_MAX = 1;     // 全 0/空数据时轴上限兜底，避免除零（折线退化为底部平线）

@Component
export struct LineChart {
  @Prop labels: string[] = [];
  @Prop series: ChartSeries[] = [];
  // version 声明在 labels/series 之后：同一帧属性按声明顺序更新，保证重绘读到新数据
  @Prop @Watch('onVersionChange') version: number = 0;
  // 双轴开关：series 前半左轴（¥），后半右轴（Tokens）
  @Prop dualAxis: boolean = false;
  // 左轴上限：未指定或 <= 0 时按数据自适应（静态配置，父组件创建后不变）
  yMax?: number;
  // 触摸命中的数据索引，-1 表示无提示；抬手后保留最后提示不清除
  @State @Watch('onTooltipChange') tooltipIndex: number = -1;
  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);

  onVersionChange(): void {
    // 数据变化后旧索引可能对应错误日期：重置提示（置位会再触发一次重绘，代价可接受）
    if (this.tooltipIndex !== -1) {
      this.tooltipIndex = -1;
    }
    this.draw();
  }

  onTooltipChange(): void {
    this.draw();
  }

  build() {
    Canvas(this.ctx)
      .width('100%')
      .height('100%')
      .onReady(() => this.draw())
      .onTouch((event: TouchEvent) => this.handleTouch(event))
  }

  // 触摸命中：Down/Move 换算最近数据索引；Up/Cancel 保留最后提示
  private handleTouch(event: TouchEvent): void {
    if (event.type !== TouchType.Down && event.type !== TouchType.Move) {
      return;
    }
    const n = this.labels.length;
    if (n === 0 || event.touches.length === 0) {
      return;
    }
    const layout = this.computeLayout();
    if (layout.plotW <= 0) {
      return;
    }
    // 触点 x → 数据索引：绘图区外先夹取到边界，单点序列恒命中 0
    const ratio = (event.touches[0].x - layout.plotX) / layout.plotW;
    const clamped = Math.min(1, Math.max(0, ratio));
    const index = n === 1 ? 0 : Math.round(clamped * (n - 1));
    if (index !== this.tooltipIndex) {
      this.tooltipIndex = index;
    }
  }

  private draw(): void {
    const layout = this.computeLayout();
    if (layout.w <= 0 || layout.h <= 0) {
      return; // 尺寸未就绪（onReady 前），由 onReady 补绘
    }
    this.ctx.clearRect(0, 0, layout.w, layout.h);
    this.ctx.setLineDash([]); // 重置上一帧残留虚线状态
    if (this.labels.length === 0) {
      this.drawEmpty(layout);
      return;
    }
    const left = this.leftCount();
    const leftMax = this.resolveAxisMax(0, left, true);
    const rightMax = this.resolveAxisMax(left, this.series.length, false);
    this.drawGrid(layout);
    this.drawYTicks(layout, leftMax, rightMax);
    this.drawXLabels(layout);
    this.drawAllSeries(layout, left, leftMax, rightMax);
    if (this.tooltipIndex >= 0 && this.tooltipIndex < this.labels.length) {
      this.drawTooltip(layout, left);
    }
  }

  // 空数据：居中"暂无数据"
  private drawEmpty(layout: ChartLayout): void {
    this.ctx.font = '12px monospace';
    this.ctx.fillStyle = Theme.muted;
    this.ctx.textAlign = 'center';
    this.ctx.textBaseline = 'middle';
    this.ctx.fillText('暂无数据', layout.w / 2, layout.h / 2);
  }

  // 左轴序列数：单轴全部在左；双轴取前半（奇数时左侧多一条）
  private leftCount(): number {
    if (!this.dualAxis) {
      return this.series.length;
    }
    return Math.ceil(this.series.length / 2);
  }

  // 轴上限：allowOverride 仅左轴生效（yMax 指定时覆盖）；全 0/空兜底 1
  private resolveAxisMax(from: number, to: number, allowOverride: boolean): number {
    if (allowOverride) {
      const override = this.yMax;
      if (override !== undefined && override > 0) {
        return override;
      }
    }
    const max = this.dataMax(from, to);
    return max > 0 ? max : AXIS_FALLBACK_MAX;
  }

  // 指定序列区间内的最大有效值（null/缺失/非有限数不参与）
  private dataMax(from: number, to: number): number {
    let max = 0;
    for (let si = from; si < to; si++) {
      const s = this.series[si];
      for (let i = 0; i < this.labels.length; i++) {
        const v = this.valueAt(s, i);
        if (v !== null && v > max) {
          max = v;
        }
      }
    }
    return max;
  }

  // 横向网格线：4 格共 5 条
  private drawGrid(layout: ChartLayout): void {
    this.ctx.strokeStyle = Theme.gridH;
    this.ctx.lineWidth = 1;
    this.ctx.beginPath();
    for (let i = 0; i <= Y_DIVISIONS; i++) {
      const y = layout.plotY + layout.plotH * i / Y_DIVISIONS;
      this.ctx.moveTo(layout.plotX, y);
      this.ctx.lineTo(layout.plotX + layout.plotW, y);
    }
    this.ctx.stroke();
  }

  // y 轴刻度文本：左轴 ¥ toFixed(2)；双轴时右轴 formatTokens
  private drawYTicks(layout: ChartLayout, leftMax: number, rightMax: number): void {
    this.ctx.font = '9px monospace';
    this.ctx.fillStyle = Theme.muted;
    this.ctx.textBaseline = 'middle';
    this.ctx.textAlign = 'right';
    for (let i = 0; i <= Y_DIVISIONS; i++) {
      const value = leftMax * (Y_DIVISIONS - i) / Y_DIVISIONS;
      const y = layout.plotY + layout.plotH * i / Y_DIVISIONS;
      this.ctx.fillText('¥' + value.toFixed(2), layout.plotX - 6, y);
    }
    if (this.dualAxis) {
      this.ctx.textAlign = 'left';
      for (let i = 0; i <= Y_DIVISIONS; i++) {
        const value = rightMax * (Y_DIVISIONS - i) / Y_DIVISIONS;
        const y = layout.plotY + layout.plotH * i / Y_DIVISIONS;
        this.ctx.fillText(formatTokens(value), layout.plotX + layout.plotW + 6, y);
      }
    }
  }

  // x 标签抽稀至 ≤ 8 个，等步长取前部索引
  private drawXLabels(layout: ChartLayout): void {
    const n = this.labels.length;
    const step = Math.max(1, Math.ceil(n / MAX_X_LABELS));
    this.ctx.font = '9px monospace';
    this.ctx.fillStyle = Theme.muted;
    this.ctx.textAlign = 'center';
    this.ctx.textBaseline = 'top';
    for (let i = 0; i < n; i += step) {
      this.ctx.fillText(this.labels[i], this.xAt(layout, i), layout.plotY + layout.plotH + 6);
    }
  }

  // 逐序列折线 + 数据点；null 断开，dashed 用 [6,4] 虚线
  private drawAllSeries(layout: ChartLayout, left: number, leftMax: number, rightMax: number): void {
    for (let si = 0; si < this.series.length; si++) {
      const s = this.series[si];
      const axisMax = (this.dualAxis && si >= left) ? rightMax : leftMax;
      this.ctx.strokeStyle = s.color;
      this.ctx.lineWidth = 1.5;
      this.ctx.setLineDash(s.dashed === true ? [6, 4] : []);
      this.ctx.beginPath();
      let penDown = false;
      for (let i = 0; i < this.labels.length; i++) {
        const v = this.valueAt(s, i);
        if (v === null) {
          penDown = false; // null 断开，下一有效点重新落笔
          continue;
        }
        const x = this.xAt(layout, i);
        const y = this.yFor(layout, v, axisMax);
        if (penDown) {
          this.ctx.lineTo(x, y);
        } else {
          this.ctx.moveTo(x, y);
          penDown = true;
        }
      }
      this.ctx.stroke();
      // 数据点圆 r=2（单点序列由此可见；先复位虚线避免状态残留）
      this.ctx.setLineDash([]);
      this.ctx.fillStyle = s.color;
      for (let i = 0; i < this.labels.length; i++) {
        const v = this.valueAt(s, i);
        if (v === null) {
          continue;
        }
        this.ctx.beginPath();
        this.ctx.arc(this.xAt(layout, i), this.yFor(layout, v, axisMax), POINT_RADIUS, 0, Math.PI * 2);
        this.ctx.fill();
      }
    }
    this.ctx.setLineDash([]);
  }

  // 触摸提示：命中索引竖虚线 + 顶部浮层（日期 + 各序列 name: value），null 值显示 "-"
  private drawTooltip(layout: ChartLayout, left: number): void {
    const index = this.tooltipIndex;
    const x = this.xAt(layout, index);
    // 竖向虚线
    this.ctx.strokeStyle = Theme.line;
    this.ctx.lineWidth = 1;
    this.ctx.setLineDash([4, 4]);
    this.ctx.beginPath();
    this.ctx.moveTo(x, layout.plotY);
    this.ctx.lineTo(x, layout.plotY + layout.plotH);
    this.ctx.stroke();
    this.ctx.setLineDash([]);

    // 浮层内容：首行日期，其后每序列一行（左轴 ¥ toFixed(2)、右轴 formatTokens）
    const lines: TooltipLine[] = [{ text: this.labels[index], color: Theme.ink }];
    for (let si = 0; si < this.series.length; si++) {
      const s = this.series[si];
      const v = this.valueAt(s, index);
      let text: string;
      if (v === null) {
        text = '-';
      } else if (this.dualAxis && si >= left) {
        text = formatTokens(v);
      } else {
        text = '¥' + v.toFixed(2);
      }
      lines.push({ text: `${s.name}: ${text}`, color: s.color });
    }

    // 浮层尺寸：按最长行测量 + 四周内边距
    this.ctx.font = '10px monospace';
    const padX = 8;
    const padY = 6;
    const lineH = 14;
    let boxW = 0;
    for (const line of lines) {
      boxW = Math.max(boxW, this.ctx.measureText(line.text).width);
    }
    boxW += padX * 2;
    const boxH = lines.length * lineH + padY * 2;

    // 位置：默认竖线右侧，右缘放不下翻到左侧，最后夹取在画布内
    let boxX = x + 8;
    if (boxX + boxW > layout.w - 4) {
      boxX = x - 8 - boxW;
    }
    boxX = Math.max(4, Math.min(boxX, layout.w - boxW - 4));
    const boxY = layout.plotY;

    this.ctx.fillStyle = Theme.surfaceRaised;
    this.ctx.strokeStyle = Theme.line;
    this.ctx.lineWidth = 1;
    this.ctx.beginPath();
    this.ctx.rect(boxX, boxY, boxW, boxH);
    this.ctx.fill();
    this.ctx.stroke();

    this.ctx.textAlign = 'left';
    this.ctx.textBaseline = 'middle';
    for (let li = 0; li < lines.length; li++) {
      const line = lines[li];
      this.ctx.fillStyle = line.color;
      this.ctx.fillText(line.text, boxX + padX, boxY + padY + lineH * li + lineH / 2);
    }
  }

  private computeLayout(): ChartLayout {
    const w = this.ctx.width;
    const h = this.ctx.height;
    const marginRight = this.dualAxis ? MARGIN_RIGHT_DUAL : MARGIN_RIGHT_SINGLE;
    return {
      w: w,
      h: h,
      plotX: MARGIN_LEFT,
      plotY: MARGIN_TOP,
      plotW: Math.max(0, w - MARGIN_LEFT - marginRight),
      plotH: Math.max(0, h - MARGIN_TOP - MARGIN_BOTTOM),
    };
  }

  // 数据索引 → x 坐标；单点序列居中
  private xAt(layout: ChartLayout, index: number): number {
    const n = this.labels.length;
    if (n <= 1) {
      return layout.plotX + layout.plotW / 2;
    }
    return layout.plotX + layout.plotW * index / (n - 1);
  }

  // 数值 → y 坐标；axisMax 已保证 > 0，越界值画到绘图区外不额外裁剪
  private yFor(layout: ChartLayout, value: number, axisMax: number): number {
    return layout.plotY + layout.plotH * (1 - value / axisMax);
  }

  // 序列第 index 个有效值：data 短于 labels 的缺失段、null、非有限数一律视为 null
  private valueAt(s: ChartSeries, index: number): number | null {
    if (index >= s.data.length) {
      return null;
    }
    const v = s.data[index];
    if (v === null || !Number.isFinite(v)) {
      return null;
    }
    return v;
  }
}
