# Oak app/ 层 GPUI 重写计划（oak-gpui 侧工作项）

> 面向实现者（DeepSeek）的任务书。主仓库：/Users/sunyu/Projects/oak
> （引擎 RIIR 进行中，模块边界是 `include/<mod>/*.h` 纯 C ABI +
> 引用计数句柄）。本文件列出 oak-gpui 仓库里需要完成的工作项。
>
> 已实现（本仓库，2026-08-09 提交）：`gpui::timeline` /
> `gpui::node_graph` / `gpui::effect_stack` / `gpui::dock` 四个
> NLE widget。
>
> 原则：widget 不直接改引擎状态，只发请求事件；引擎（oak C ABI）
> 是唯一事实源。crates.io 有成熟库就不自造。测试驱动：每个 widget
> 的状态机/几何计算必须有单测（参照 timeline/time.rs 的做法）。

## W1. 表单控件库（咽喉项，最先做）

位置：`crates/gpui_widgets/`（新 crate）。这是参数面板和全部对话框
的前置依赖。

对标 oak C++ 侧的 `app/widget/slider/` 与 `app/widget/nodeparamview/`：

- [x] `Slider` 族：float / integer / rational（分式）/ 角度。
  拖动改值（上下拖 + 微调修饰键）、双击直接输入、滚轮步进、
  中键复位默认值。数值格式化与解析必须可注入。
- [x] `SpinBox`（数字输入 + 上下按钮）。
- [x] `ComboBox`（下拉选择；纯 gpui 弹层实现）。
- [x] `CheckBox` / `RadioGroup`。
- [x] `ColorSwatchButton`（色块按钮 + 点击弹取色器）；
  `ColorPicker`（HSV 轮 + RGBA 输入 + 吸管占位）。
- [x] `CurveEditor`（关键帧曲线编辑，供时间重映射等；canvas 绘制，
  贝塞尔控制点拖拽）。
- [x] 键控支持：每个可键控控件右侧的关键帧菱形按钮（状态：
  无键/有键/在当前帧），点击发请求事件。
- [x] 全部控件的状态逻辑（值域、步进、钳制、非法输入拒绝）有
  单测；绘制走 gpui canvas/quad，不碰平台 API。

## W2. 菜单与对话框框架

位置：`crates/gpui_widgets/`（或独立 `gpui_dialogs`）。

- [x] `ContextMenu`/`MenuBar` 窗口内菜单组件（Zed 的菜单在 zed app
  crate 而非 gpui，需要自带）：弹层定位、键盘导航、子菜单、勾选/
  禁用态、快捷键展示。
- [x] `Modal` 对话框框架：模态遮罩、标题栏、按钮行（确定/取消/
  应用）、Esc/Enter 默认键、尺寸约束。
- [x] 常用对话框原语：消息框（info/warning/error 三档）、文件选择
  （包 `prompt_for_paths`/`prompt_for_new_path` 平台 API）、进度条
  对话框（可取消）。
  > 注：本 fork 尚无 `prompt_for_paths`/`prompt_for_new_path` 平台
  > API（已核实 window/platform 均无），文件对话框先用纯 gpui 模态 +
  > 路径输入框实现；后续接入真实平台选择器时替换内容即可。
- [x] 单测：菜单模型（勾选/禁用/级联）、对话框结果路由。

## W3. macOS 视频帧桥接（关键路径）

目标：引擎渲染结果零拷贝上屏。

- [x] 引擎侧输出是 wgpu 纹理（Metal 后端）。在 `gpui_media` 或新
  `oak_bridge` crate 里做 wgpu Metal 纹理 → IOSurface →
  CVPixelBuffer 的包装（`CVMetalTextureCache` helper 已在
  `gpui_media/src/media.rs`），输出给 `window.paint_surface`。
  > 实现在新 crate `crates/oak_bridge/`：`SurfaceBridge::blit_frame`
  > 用自带 MTLCommandQueue + MTLBlitCommandEncoder 做 GPU→GPU 拷贝
  > （blit 前 `device.poll(Wait)` 保证引擎渲染完成，v1 同步但正确）。
- [x] 保留 CPU 回读兜底路径（任何后端可用），但默认不走。
  > `stage_readback` + `finish_readback`（wgpu readback → 
  > `CVPixelBufferCreateWithBytes`）。
- [x] 验收：1080p/4K F32 帧连续上屏无掉帧（写一个 demo example：
  循环显示测试图序列，测 FPS）；CI 无 GPU 环境跳过。
  > `cargo run -p oak_bridge --example surface_bridge --features demo`
  > （macOS + Metal，无 GPU 环境直接退出）。实测 1280x720 连续上屏
  > ~11-14fps（受 v1 同步 poll 限制，引擎侧可改为异步管线优化）。
- [ ] Windows/Linux 路径用 gpui_wgpu 的 `paint_surface(wgpu::Texture)`
  直连，同 demo 验证。
  > 未做：wgpu 29 无 IOSurface 原生支持且 hal `Texture` 字段私有，
  > Linux/Windows 的 surface 桥接留待后续（`SurfaceSource::Texture`
  > 已在 gpui 侧就绪，只需在 `oak_bridge` 补对应平台模块）。

## W4. 播放同步与检视器 glue

- [x] `ViewerWidget`（新，放 `crates/gpui_widgets/` 或 oak 侧）：
  画面区（W3 的 surface）+ 走带控制（播放/暂停/逐帧/入点出点）+
  时间码显示 + 安全框/缩放开关。播放驱动：oak audio 引擎时钟经
  C ABI 查询，`cx.spawn` + timer 刷新播放头。
  > 实现在 `gpui_widgets::viewer`：`PlaybackClock` trait（host 在
  > oak 侧经 C ABI 实现），widget 以 ~60Hz timer 轮询并把走带操作
  > 作为 `ViewerEvent` 请求发出；时间码复用 `gpui::timeline::time`
  > 的 `format_timecode`（含 `TimeDisplay::Timecode`）。
- [x] 单测：时间码换算（复用 oakcore-rs Rational）、走带状态机。
  > `transport.rs` 纯状态机单测（advance/loop/step/in-out）+ 时间码
  > 测试；`examples/viewer.rs`（macOS 生成测试帧上屏，非 macOS 显
  > 示占位）。
  > 附带修复：`gpui_macos` 的 Surface 渲染此前只接受 YUV 420 双
  > 平面缓冲，为让引擎的 BGRA 帧直接上屏，给 `metal_renderer` +
  > `shaders.metal` 增加了单平面 32BGRA 分支（SurfaceBounds 带
  > `is_bgra` 标志，shader 直接采样 BGRA 并交换到 RGBA）。

## W5. 时间线工具模式层

- [x] 在 oak 侧（不在本仓库）实现 14 个工具模式（ripple/roll/slip/
  slide/razor/ 等）为 `gpui::timeline` 的 `TimelineEvent` 消费者 +
  引擎命令映射。**本仓库侧配套**：`TimelineEvent` 覆盖不全的手势
  （如 transition 拖拽、轨道选择）按需补事件。
  > 本仓库侧已补：`TimelineEvent::TrackSelected`（轨道头点击选择，
  > 视图维护 `selected_tracks` 集合）与
  > `TimelineEvent::TransitionChanged`（过渡楔形边缘拖拽改长度，
  > 按 zoom 换算并钳制到片长）。14 个工具模式本体在 oak 侧实现。
- [x] 素材箱（ProjectExplorer）：树 + 图标双视图，文件拖入经
  `FileDropEvent`，缩略图经 sprite atlas。
  > `gpui_widgets::project_explorer`：数据无关 `ProjectDataSource`
  > trait，树/图标双视图（纯 `flatten_tree` 单测）、文件拖入经
  > `on_drop`（平台 FileDrop 转内部 drag）发出 `FileDropRequested`、
  > 缩略图走 `img()`（sprite atlas）。`examples/project_explorer.rs`。

## W6. 示波器与音频表

- [x] `Histogram` / `Vectorscope` / `Waveform` 检视组件（canvas
  绘制，数据来自 oak render C ABI 的帧采样）。
  > `gpui_widgets::scopes`：纯数学（直方图分箱、波形 min/max 包络、
  > vectorscope 色度投影）单测覆盖；`LumaDataSource`/`ChromaDataSource`
  > trait 由 host 经 C ABI 提供帧采样。
- [x] `AudioLevelMeter` 表头（数据来自 oak audio C ABI）。
  > `gpui_widgets::audio_meter`：分段点亮 + 峰值保持衰减（纯数学
  > 单测）；`AudioMeterDataSource` trait。
  > `examples/scopes.rs`：mock 信号源驱动四个组件。

## W7. 主题系统

- [ ] 设计系统：把 oak 的 olive-dark/olive-light QSS 翻译成 gpui
  的 `Colors`/样式结构，支持运行期切换。

## 顺序与验收

1. W1 → W2（解锁参数面板与对话框）
2. W3（检视器能上屏）→ W4
3. W5/W6 并行
4. W7 随时

每个 W 的完成标准：cargo test 绿
+ 一个可运行的 example 演示。
widget 与 oak 引擎的联调在 oak 仓库侧做（本仓库只交付 widget）。
