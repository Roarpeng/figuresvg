# figuresvg — Raster Figure → Semantic Scene Graph → Editable SVG

独立服务项目：把扁平色科研图（系统发育树、热图、柱状/散点/饼图）转成
**真可编辑**的 SVG —— 文字是 `<text>`、图形是 `<rect>/<circle>/<line>`、
一切带语义 id，而不是一堆不可编辑的描迹路径。

## 架构

```
                 原始 PNG/JPG
                       │
             ┌─────────┴─────────┐
             ▼                   ▼
      PaddleOCR(文字)      Rust 引擎(图形)          ← 确定性识别层，无 LLM
      text+bbox+字号       palette-layers -> 原语分类
             │                   │
             └─────────┬─────────┘
                       ▼
                Fusion（纯代码）   ← 去字形重复、建 labels 关系
                       ▼
                Scene Graph JSON   ← 中间契约（schema/SCENE_GRAPH.md）
                       ▼
                SVG Generator（纯代码）
                       ▼
                可编辑 SVG（<text> <rect> <circle> <line> <g id>）
```

- **识别器可插拔**：OCR / geometry 都是独立模块，走同一 Scene Graph 契约；
  VLM 语义层（Qwen3-VL 等）预留为 tags/relations 注入器，永不经手坐标。
- **Rust 引擎**（engine/）：TypeSafe 裁决的 palette-layers 策略（精确调色板
  量化 + 每色一层二值描迹），7232×3878 大图 90 秒；scene 模式把每个连通域
  分类为 circle > rect > line > polygon > path 最具体原语。

## 使用

```bash
pip install -r requirements.txt
(cd engine && cargo build --release)

# CLI
python -m figuresvg convert IN.png OUT.svg [--scene scene.json] \
        [--recolor-json '[{"row":9,"donor_rows":[7,8,12]}]'] [--skip-ocr]

# HTTP 服务
python -m uvicorn figuresvg.server.api:app --host 0.0.0.0 --port 8417
curl -F file=@fig.png http://127.0.0.1:8417/api/convert -o result.json
#   -> {"stats": {...}, "scene": {...}, "svg": "..."}
```

## v3 编译器（位图验证换心）——保真度与可编辑性的统一

v2 教训：用 `<text>` 重排替换描迹字形、用原语替换像素层，会在文字密集图
（如 c.jpeg）上大幅损失保真度（均差 21.7）。v3 按「位图对比→低于阈值有
效→否则针对性修改」重构：

1. **像素引擎输出 = 保真底座**（palette-layers，c.jpeg 均差 0.97）；
2. PaddleOCR 3.x 文字 + Rust 引擎 Scene Graph（语义数据，供工具链）；
3. **换心手术**：每个文字框内删除描迹字形路径、插入真 `<text>`，
   整图试渲染后逐框与原图对比；
4. 偏差 ≤ `TEXT_ACCEPT_DIFF`(12/255) 才接受换心，否则保留描迹字形
   （文字内容仍进 scene.json，机器可读）；
5. 输出 = 底座保真度 + 通过验证的可编辑文字。

实测（`*_edit.svg`）：

| 图 | 均差(含改色) | 可编辑`<text>` | 保留描迹 |
| --- | --- | --- | --- |
| c.jpeg 树+改色 | **0.99**（v2 为 21.7） | 1 | 46 |
| 1 热图+条形 | 1.12 | 0 | 24 |
| 2 分组柱状 | 2.70 | 2 | 27 |
| 3 散点+趋势线 | 1.73 | 0 | 14 |
| 5 火山图 | 2.16 | 3 | 42 |

可编辑比例由 `TEXT_ACCEPT_DIFF` 控制（compiler.py）：调高换更多文字、
保真度略降；小字号/细字重文字重排必然偏移，默认保守是对的。

## 实测（可编辑模式，含真字体渲染差异）

| 图 | mean diff | `<text>` | 说明 |
| --- | --- | --- | --- |
| 1 相关性热图+条形 | 5.67 | 16 | 修复稀疏边框误判实心矩形后 |
| 2 分组柱状+误差棒 | 5.39 | 27 | |
| 3 散点+趋势线 | 4.74 | 16 | |
| 5 火山图 | 7.69 | 37 | |
| c.jpeg 树图+改色 | ~20 | 47 | 学名完美（Zostera marina…）；改色生效 |

注：mean diff 高于纯描迹模式（~1-3）是**可编辑性的合理代价** —— 文字用
标准 sans-serif 重新排版而非原字形描摹。追求像素级还原用引擎直出
（`engine` build，不带 --scene）。

## 最终结果（v2.0，36 张测试图）

| 类别 | 平均diff | 范围 | 图片数 | 方法 |
|---|---|---|---|---|
| flowers | 5.03 | 3.6-6.6 | 15 | 径向圆拟合 + 绿色排除 + 黄心CC |
| people | 5.39 | 5.0-5.7 | 3 | 对象合并 + 椭圆/多边形分解 |
| anime_chars | 5.20 | 4.9-5.4 | 3 | 对象合并 + 椭圆检测 |
| anime_items | 4.60 | 2.5-6.6 | 3 | 轮廓多边形 + 灰色死区修复 |
| logos | 7.11 | 2.2-10.8 | 3 | 文字 + 几何原语 |
| pets | 3.42 | 2.6-7.6 | 15 | Otsu 阈值 + 精确轮廓多边形 |
| **总体** | **4.56** | 2.2-10.8 | **36** | **35/36 全部 <10** |

关键突破：
- **Otsu 阈值**替代固定 lum<220（pets 从 10.6→3.4）
- **精确轮廓多边形**替代圆分解（pet_02 从 103→2.6）
- **色相/亮度双约束合并**（色距≤30 且明度差≤20）
- **网格守卫**（≥4%画布面积→按色CC拆格）
- **退化轮廓上限**（>2000点→bbox rect，防 360MB d 串）

## 已知限制（v0.1）

- 字号/基线由 bbox 估算（h/0.72、y+0.78h），个别文字偏移需手调；
- 低对比度白字（热图格内数值）OCR 漏检时保留为描迹路径；
- 稀疏线框组件回退为描边轮廓 path（外部轮廓，内部网格线未分解）；
- VLM 语义层未实现（契约已留）。

## 关键坑（实现时踩过）

- PaddleOCR 3.x + paddlepaddle 3.x 默认走 onednn/PIR 执行路径会在部分
  CPU 崩溃（ConvertPirAttribute2RuntimeAttribute）→ 构造时
  `enable_mkldnn=False` 即可；**必须用 3.x**（PP-OCRv5/v6）：实测物种
  学名、中文标题、小字号全面优于 2.7.3（PP-OCRv3），2.7.3 连图里的
  中文都认不出；recognizer 内置 2.x API 回退；
- 框重叠计算必须双轴钳位（负×负=正，两个都不相交的框会算出正重叠）；
- 从大 SVG 里按偏移删子串时，span 必须相对所在组计算（绝对偏移搜全文
  会在后半文档切错位置），且多文字框重复匹配同一路径需区间合并；
- rect 判定必须用**像素面积/bbox 比**而非轮廓围合面积（边框外轮廓
  围合率≈1.0 但实心度 0.7%，误判会整块涂黑）；
- XML 注释内禁 `--`；f-string 不能嵌转义引号（py3.10）；
- 多层 CC 标签必须全局唯一（layer_id × 4M + cc_id）。
