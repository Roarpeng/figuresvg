# figuresvg v2.0 技术实现方案
## 目标：任意扁平色科研位图 → Origin/Inkscape 级结构化 SVG

验收对照物：`Relative_gh31.svg`（Origin 导出）+ `path3.svg`（Inkscape 绘制）。
终验标准：**把 Relative_gh31.svg 渲染回位图 → 走本流水线 → 产物与其原
结构比对**（对象数、类型、文字、刻度节律），这是天然的闭环 oracle。

---

## 0. 数据流总览

```
位图 ──► 预处理（去噪/白化，已有 Rust clean_noise）
   ┌────────────────────────────────┐
   │ 文字通道                        │
   │ 三向 OCR(0°/±45°/90°) ─ 文字元素 │
   │ 字体校准(已有 fontcal)           │
   │ 墨迹对齐落位(新，替代 OCR 回环)   │
   └───────────────┬────────────────┘
   ┌───────────────┼────────────────┐
   │ 对象通道（Rust，核心新工程）      │
   │ 调色板量化(已有)                 │
   │ ► 对象合并(新：并查集+色相连续)   │
   │ ► 形状定型 v2(新：最小二乘拟合)   │
   │ ► 笔画聚组(新：误差棒/刻度/轴)    │
   │ ► 渐变对象(新：linearGradient)   │
   └───────────────┬────────────────┘
                   ▼
        Scene Graph v2（文字排除区注入对象层）
                   ▼
        Origin/Inkscape 式 SVG 编译器（新）
        mm 单位 / layer 分组 / 全 id / style 属性 / tspan / rotate
                   ▼
        闭环验证（已有 verify 扩展对象比对）
```

## 1. 对象合并（Rust，`objects.rs` 新模块）——本方案的心脏

**问题**：当前一根柱子 = 十几个调色板 bin 碎片。参照文件要求一对象一元素。

**算法**：
1. 输入：量化后的像素→调色板索引图（已有）+ 各 bin 的连通域
2. 建图：节点=连通域；边=（膨胀 2px 后相邻）且（色距 ≤ 24，RGB 最大
   通道差）且（合并后轮廓仍为简单闭合，无白缝穿越）
3. 并查集合并 → 视觉对象（像素集合）
4. **白缝守卫**：两域间若存在 ≥2px 纯白间隔则永不连边（热图格子安全）
5. 文字排除区（OCR 框膨胀 2px）内的域不参与对象层，归文字层处理

**输出**：对象列表（像素掩码 + 主色 + 色彩跨度）

## 2. 形状定型 v2（Rust，`geom` 升级：拟合代替启发式）

对每个对象掩码，按残差升序尝试，取首个通过者：

| 原语 | 拟合方法 | 接受残差 |
|---|---|---|
| `circle` | 最小二乘圆拟合（质心初值） | 轮廓点径向偏差均值 / r < 0.10 |
| `rect` | 掩码 → 最小外接矩形（含圆角检测：角部缺墨比 ≈ rx） | 像素填充率 ≥ 0.96 |
| `line` | 骨架主轴 PCA → 两端点 | 细长比 ≥ 8:1 |
| `polyline` | 骨架折线化（Douglas-Peucker ε=1.5px） | 笔画型掩码 |
| `polygon` | 外轮廓 DP 简化 ≤ 12 顶点 | 面积保持 ≥ 0.97 |
| `path` | 兜底：vtracer 描迹（仅复杂轮廓） | — |

**样式提取**：stroke_width = 笔画掩码的平均厚度（距离变换均值×2）；
stroke 颜色 = 对象边缘色 vs 内部色分离（有描边时边缘环颜色一致）。

## 3. 笔画聚组（`strokes.rs` 新模块）

- **误差棒**：竖直短线 + 上下两条水平短帽，中心对齐、总高 < 60px、
  与某柱/点对象 x 中心差 < 4px → 聚为 `errbar_N`（g 组内 3 条 polyline）
- **刻度**：轴线上等间距排列的平行短线 → 按长度分主/次刻度，
  各自独立 polyline（Origin 同款）
- **轴**：绘图区边界上的最长直线 → 1 条主 polyline
- 分组产物进 Scene Graph 的 group 字段

## 4. 渐变对象（`gradients.rs`）

对象内部色跨度 > 40 且沿某轴单调（Spearman > 0.9）→ 输出
`<linearGradient>` def（两端+中点采样 3~5 个 stop），对象引用之。
非单调（如红白蓝 colorbar）→ 分段多个渐变或退回 bin（c.jpeg colorbar）。

## 5. 文字层终版（Python 侧改造）

1. **三向 OCR**：图像旋转 0°/45°/90°/-45° 各跑一次，按文字行主方向
   归并去重；旋转文字记录角度
2. **墨迹对齐落位**（替代 OCR 回环定位，前轮已定位的杠杆）：
   源文字墨迹框 vs 校准字体渲染墨迹框的偏移直接回填 dx/dy——一次到位
3. **编译格式**：`<text text-anchor x y transform="rotate(θ,cx,cy)"
   style="font:...">` + `<tspan>`，字体链（fontcal 已有）

## 6. SVG 编译器（`compiler2.py` 新）

- 根：`width="{W}mm" height="{H}mm" viewBox="0 0 W_px H_px"`（默认
  300DPI 换算，spec 可覆盖）
- `<g id="layer1_objects">` / `<g id="layer2_texts">` / 图例
  `<g id="legend_N">`（组内 swatch+label，已有逻辑）
- 每元素：id、style 属性化（fill/stroke/stroke-width/opacity）、
  `shape-rendering="crispEdges"`（直边对象）
- mask：检测绘图区边界时输出 `<defs><mask>`（Origin 同款，可选开关）

## 7. 验证（`verify.py` 扩展）

现有闭环（重渲染→重 OCR→文字比对）+ 新增：
- **对象比对**：重渲染图重跑对象通道，对象数/类型直方图/平均 IoU ≥ 0.8
- **参照回环**：Relative_gh31.svg → cairosvg 渲染成位图 → 全流水线 →
  与原 SVG 结构 diff（对象数 ±10%、文字 100%、刻度组节律一致）

## 8. 分阶段计划与验收（合计 ~5 个工作日）

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| S0 (0.5d) | 编译器骨架 + Scene Graph v2 字段 | 2.png 全链跑通，Inkscape 打开显示正确图层名 |
| S1 (1.5d) | 对象合并（Rust） | **2.png：5 根柱=5 个 polygon**，对象总数 < 200（现数千）；渲染差 ≤ 4 |
| S2 (1d) | 形状定型 v2 + 样式 | 闭环对象比对类型一致率 ≥ 90%（1/2/3/5） |
| S3 (0.5d) | 笔画聚组 | 2.png 误差棒聚组正确、刻度独立 polyline |
| S4 (0.5d) | 渐变对象 | c.jpeg 条带=渐变矩形（对象数 < 40），带区渲染差 ≤ 3 |
| S5 (0.5d) | 文字终版（三向 OCR+墨迹落位） | 闭环 text_match ≥ 0.7（1/2.png） |
| S6 (0.5d) | 参照回环 + 全套回归 | Relative_gh31 回环达标；全套交付 |

## 9. 风险与对策

| 风险 | 对策 |
|---|---|
| 合并过度（热图格子粘连） | 白缝守卫 + 合并轮廓简单性检查；回环对象比对兜底 |
| 渐变方向非轴对齐 | PCA 主轴决定 linearGradient 的 gradientTransform |
| 45° 文字 OCR 漏检 | 三向扫描 + 文字区残留墨迹告警（进 worst report） |
| fill-opacity 不可逆 | 默认实心；仅重叠混色证据时输出（已向用户声明） |
| c.jpeg 28MP 性能 | 合并/拟合全在 Rust；bbox 裁剪原则沿用 |

## 10. 代码落点

```
figuresvg/
  engine/src/objects.rs      # 新：对象合并
  engine/src/strokes.rs      # 新：笔画聚组
  engine/src/gradients.rs    # 新：渐变检测
  engine/src/geom.rs         # 升级：最小二乘拟合
  engine/src/main.rs         # scene 模式改为对象输出
  figuresvg/compiler2.py     # 新：Origin/Inkscape 式编译
  figuresvg/recognizers/ocr.py  # 三向扫描
  figuresvg/placement.py     # 新：墨迹对齐落位
  tests/test_roundtrip.py    # 新：参照回环 oracle
```

## 实施进度（2026-09-20 第一轮）

- ✅ S0 编译器 v2：mm 单位/layer1_objects+layer2_texts/全 id/style 属性/
  crispEdges/tspan/rotate——2_edit.svg 头部与参照文件同构
- ✅ S1 对象合并：2.png 柱=18 个单 rect（17×261 级）；边局部色距判据 +
  平板守卫（≥8% 画布面积→按色 CC 拆格，无缝热图恢复 69 格）+ 白缝守卫
- ✅ S1b 黑色笔画对象层：误差棒/轴/刻度不再被丢弃（24 个误差棒形态）
- ✅ S5 部分：字体校准 + 墨迹对齐落位（ink_placed 28/29）
- 实测：2.png 4.01/0.937、3.png 3.06/0.62（text_match 0.643）
- ⚠️ 1.png 21.5/0.45（热图格子恢复一半，另一半待查）；5.png 13.9/0.37
  （密集散点碎片化）
- ⏳ S3 笔画聚组、S4 渐变接线、S6 参照回环：未开始
- 工程坑：cargo 增量缓存偶发不重编（rm fingerprint 强制）；sub.len()==1
  单色捷径会绕过合并守卫（平板守卫需双路径覆盖）

## 实施进度（2026-09-20 第二轮）

- ✅ 1.png 热图修复：113 个格子矩形（grid-cell 直发 bbox rect，跳过描迹）
  ——覆盖 0.45→0.845，均差 21.4→7.42
- ✅ 退化轮廓防爆：2px 斑块轮廓震荡产生 360MB d 串（SVG 720MB）→
  轮廓 >2000 点退化为 bbox rect；场景文件 720MB→167KB
- ✅ 平板守卫阈值 8%→4%（单色/合并双路径）
- ✅ S6 参照回环 oracle 首跑（Relative_gh31 → 位图 → 全链）：
  结构同级（50 对象 vs 参照 67 元素；柱→rect/polygon、刻度→line）、
  文字 6/10 精确匹配（含 45° 标签字符串 WT/gh3.1-1/gh3.1-2）；
  c→C 大小写误读、45°/90° 旋转变换未回写（S5 剩余）、y 轴数字漏检
- 数字：1=7.42/0.845、2=4.03/0.938、3=3.09/0.644(tm .643)、
  5=13.9/0.374（密集散点待专项）
- ⏳ 剩余：S5 旋转 OCR+变换回写、5.png 散点专项、S3 笔画聚组、
  c.jpeg 全链

## 游戏资产测试结果（2026-09-27）

使用 20 张游戏风格位图测试管线：
- WoW: 联盟徽章、部落标志、霜之哀伤、兽人战士、Boss血条
- Zelda: 三角力量、大师之剑、希卡之眼、心之容器、卢比、Boss钥匙
- 2077: Logo(故障效果)、数据碎片、神经连接、螳螂刀、义体界面
- 文明: 罗马建筑、科技树、资源图标、世界地图

结果：20/20 全部转换成功。使用 Rust 引擎直接 build 输出（palette-layer
tracing），平均 diff=2.15，最大 5.1。全部 <10。

关键发现：对于复杂游戏资产（暗背景+细线条+多色重叠+故障效果），
**像素级描摹**（palette-layer tracing）效果远优于**结构化分解**（scene
classification）。Scene 模式的对象分类在复杂背景下会碎裂成大量碎片。

正确的模式选择策略：简单图表/图标 → scene 模式（可编辑）；
复杂游戏资产 → build 模式（保真优先）。
