# ADR 0008：破坏 VFX 的冲击感与暴露破口边缘细节

- 状态：提议（调研完成，尚未采用或实现以下改进）
- 日期：2026-11-17
- 关联：[ADR 0007](0007-bevy-bsn-fracture-interaction-demo.md)、[ADR 0006](0006-blender-authoring-tool-and-asset-format.md)

## 背景与目标

用户要求调研怎样让 VFX 更能 sell 破坏，以及怎样增加被破坏网格的边缘细节。这里将 sell 理解为：玩家能读出冲击、碎裂、重量和材料，而不是仅知道发生了一次点击。

本记录区分已核实的代码/资料事实、本项目的设计推论和待验证参数。本轮只提交调研记录，不改运行时代码、资产格式、现有破坏范围或已采用的物理规则。

## 已核实的现状

| 现状 | 对观感的影响（设计判断） | 仓库证据 |
| --- | --- | --- |
| 每次至多 64 粒子，按索引先生成 16 碎屑、28 烟尘、20 细粉；同一帧出生 | 时间峰值重合，缺乏先冲击、再碎裂、后余韵的层次；预算不足时后面的层级可能完全缺席 | `src/vfx.rs::spawn_bursts` |
| 碎屑只有一种 Cuboid，尺寸 0.035–0.09m，自旋下落但不碰撞，随后缩小消失 | 可读形状、材质和落地反馈不足 | `src/vfx.rs` |
| 烟尘/细粉为同一张 64×64 径向软边图、无光照材质，放大并使用 16 档淡出 | 更像均匀雾斑；软边图没有解决与墙/地面的交线，也没有体积明暗 | `src/vfx.rs::setup/animate_particles` |
| 事件只有命中位置/法线、种子和最多 12 个暴露点，暴露点来自 Bond 中心，所有层都用命中法线 | 缺少材料、破坏规模、各发射面的方向与分布；侧面、墙体、楼板可能向同一方向喷 | `src/interaction.rs`、`BreakBurst` |
| 断面已经有骨料/孔隙噪声和高度梯度法线扰动，但几何仍是平面 Voronoi 单元 | 表面光照粗糙，但大破口仍有长直边和尖锐多边形角；加法线不改变真实轮廓 | `src/material.rs`、`assets/shaders/building_surface.wesl`、`fracture.py` |
| 导出只标 exterior/fracture，没有可寻址的接触 patch/洞口边界 | 无法可靠地只修改“刚暴露的那一段边” | `surfaces.py`、`ASSET_FORMAT.md` |

清单统计也显示尺度不匹配：三个资产的碎块最大轴尺寸中位数分别约为 rectangle 1.696m、l_shape 1.577m、courtyard 2.097m；质量中位数约为 524 / 476 / 955kg。这是按现有代理和清单计算的资产数据，不是实物测量。删除米级几何却只出现几厘米方块，很难表达大块材料去了哪里；VFX 不能被误称为守恒的真实碎裂模拟。

## 资料支持的方向

- Frostbite 的破坏表现将移除主体、补充破口周边 detail mesh、叠加破坏材质以及粒子/碎屑组合起来，说明几何、材质、动态效果应共同服务于破口，而不是全部寄托于烟尘。[S1]
- SideFX 的 Debris Source 从已分离的内部表面发射，并保留体积、深度、年龄等属性。这支持“从真实新暴露区域取样”，而非只从命中中心发射。[S2]
- Soft particles 通过粒子和场景深度差衰减相交区域；透明大面片的重叠和屏幕填充也是重要成本，粒子数量不是完整预算。[S3]
- Six-way lighting 用六个方向的烘焙光照响应增强烟尘的受光层次；它仍是面片近似，不能解决自由相机贴近/穿入面片时的全部问题。[S4]
- SideFX 明确区分混凝土角点 chipping、布尔 edge detail 和 interior detail，并支持单独代理；这种层级划分可借鉴到 Blender 流程，无须引入 Houdini 作为依赖。[S5–S7]

## VFX 提议：先建立因果与节奏

以下时间、数量和尺度均为本项目的首轮试调区间，不是资料给出的标准、物理测量或已经验证的默认值。仍立即提交逻辑删块，不通过延迟伤害或额外连删建筑来配合特效。

| 时间/阶段 | 表现目标 | 建议起点 |
| --- | --- | --- |
| 0–80ms：接触粉爆 | 点击当帧读到短促冲击 | 紧贴命中面的扇形细粉，快速展开；普通混凝土不默认加火球、强闪光或火花 |
| 40–350ms：主碎屑 | 看清材料离开破口，形成重量线索 | 4–8 种共享的不规则石片网格，混合少量约 5–25cm 主碎屑与细屑；形状/厚度应受源材料和薄墙尺寸约束 |
| 80–1500ms：尘团 | 用卷曲扩散承接破碎，而非遮住全部结果 | 2–4 套非圆形短序列烟尘，先快后慢扩张，随机朝向/播放速度，连续透明度曲线；约 0.3–0.6s 后逐步露出破口 |
| 0.3–3s：边缘余韵 | 让刚形成的边缘继续掉灰 | 从新暴露边和表面发射少量短时落屑；粗屑受重力，尘团以阻力、轻微上浮和低频涡动为主 |

需要同时解决以下问题：

1. **事件携带破坏上下文。** 建议增加材料类别、被删体积、暴露面积及发射面采样位置/法线。第一步可先利用现有 Chunk 和 Bond 数据得到规模及有方向的近似；后续使用真实 patch 三角面按面积采样、边界按长度采样。鼠标点击只有视觉强度，不应伪装成已经求解的能量/冲量。
2. **分离各层的分布。** 粗屑沿主冲击方向和各自暴露面法线发出，叠加受限切向扰动；烟尘沿破口展开。法线应从存活侧朝已删除侧：现有 `Bond.normal` 固定由 a 指向 b，a 存活时用原方向，b 存活时取反；真实 patch 进一步使用局部三角面法线。不能把每个接触面都按同一命中法线处理。可用 Curl noise 提供可控涡动，但简单加噪声并不自动提供避墙/碰撞能力。[S8]
3. **碎屑有几何变化和短暂落地行为。** 首版只让少量可读的主碎屑使用限额的碰撞/反弹，其他仍为装饰粒子。现有场景没有无限地面平面，不能用一个假想 Y=0 平面代替楼板、洞口和真实建筑查询。不要把所有 640 粒子变成 Avian 刚体，也不要给全部主碎屑统一缩小消失。
4. **形状、透明度和运动分别控制。** 用烟尘 flipbook/噪声轮廓替换纯径向圆斑；轻微材质颜色变化不等于随机彩色粉尘。先试廉价环境光与主光响应，主视觉尘团再考虑 six-way。
5. **预算保留每次新命中的反馈。** 对接触粉爆、主碎屑、烟尘分别设预算；额度不足时优先削减旧的低优先烟尘，为新命中保留短反馈。加入距离/屏幕面积 LOD，同时记录透明覆盖和 GPU 时间，不能只提高 640 的上限。

可最后补同步的断裂声、主碎屑落地声和可关闭的轻微视图震动。它们是表现补强，不替代几何细节；自由飞行下不建议默认强震屏、hit-stop 或自动拉动视角。

### Bevy 0.20 落点与兼容性

- 保留现有 ECS/共享网格和材质即可先验证时序、形状、方向及层级预算，不必先迁移整个粒子系统。
- Soft particles 可从相机 `DepthPrepass` 起步，在 WESL 中比较同一坐标系下的粒子/不透明场景距离，加入相交淡出与近相机淡出。Bevy 使用反向深度，近处 NDC Z=1；应使用其视空间转换函数，不能直接抄旧 DirectX 样例的比较符号。Alpha blend 不进入该 prepass；还需验证 MSAA、深度清除值、裁剪和附加 pass 成本。[S9]
- **Hanabi 0.20.0 已有 Bevy 0.20 版本兼容依据**：固定标签 README/Cargo 明确匹配，支持 GPU 模拟、网格、flipbook、拖曳等；不能继续以旧缓存的 0.19 README 判断不兼容。其碰撞功能表仍未勾选形状和深度碰撞，本次没有核实其内置 soft particles/six-way，也未在本仓库编译或跑 GPU。将它列为性能需求出现后的候选，而不是画质升级的前置条件。[S10]

## 破口边缘提议：表面层次与真实轮廓分开处理

| 方案 | 能获得的细节 | 成本/限制 | 建议 |
| --- | --- | --- | --- |
| 断面边带材质：新鲜骨料、颜色/粗糙度变化、局部法线和缝隙暗部 | 中景的厚度、层次与材料对比 | 不改变真实轮廓；应由真实边界距离控制，避免给每个三角形画线或一圈均匀黑边 | 优先做 |
| 少量附着在暴露破口的棱角石片/局部闭合边带 | 近景的轮廓突起、掉角与阴影 | 必须贴合真实边/面，处理转角及重复覆盖；附加石片不能漂浮、堵门窗或只是遮住错误接缝 | 做小范围原型 |
| 离线生成共享的不规则切割面，重分两侧外表面 | 真实锯齿轮廓、断面起伏与匹配阴影 | 显示模型可能不凸，烘焙/布尔、三重交汇与导出验证成本高 | 作为高质量几何路线 |
| 只提高法线噪声或普通 POM | 断面内部的光照/视差 | 不应视为真实破口轮廓方案 | 保留微细节用途 |
| 所有碎块统一 bevel 或独立 displacement | 便于快速得到碎石外观 | 可能破坏完整建筑的咬合、出现提前裂缝；统一圆角也容易呈现磨圆/塑料感 | 不直接采用 |

### 接触面数据与暴露状态

建议逐步增加可寻址的表面数据，暂不在本记录中冻结字段名或升级格式：

- 稳定 patch ID、两侧 Chunk、标准法线、真实三角面/边界环；必要时保留局部厚度及与原外表面相交的 rim 边。
- Bond 是结构连接；同一 Bond 可能聚合多个接触补丁。它的中心、面积与二阶矩不足以重建每个补丁，不能把 Bond ID 直接当成唯一切面身份。
- 单侧暴露条件为 `alive(self) && !alive(neighbor)`；未破坏状态不在外表面显示损伤。没有邻块的原外表面或门窗边界不能误判成新破口。
- 同一 patch 可供材质边带、局部几何和 VFX 发射共用。状态放在建筑实例，不能修改共享 mesh/material 使另一栋同资产建筑也显损伤。
- 新增 chip/边带切面继续标为 fracture，保留所属 Chunk 的材料类别；原 exterior 的网格坐标和未损伤部分保持连续，不能让格线覆盖新鲜碎裂面。
- 现有 debug 边是整块折角，不等于真正洞口边。烘焙时保留内外表面交线更可靠；SideFX Boolean Fracture 也显式输出内/外表面及外部交线，并支持切割面属性传递。[S7]

低成本材质原型可先只处理自然暴露的 fracture 面，缓存它与 exterior 的交界；这不要求先完成整个新格式。若要让材质损伤跨到外表面、按邻块选择几何或从完整接触区域发射，再补 patch 身份和逐实例暴露状态。不要因此把所有细节工作都阻塞在格式大改上。

### 几何一致性与管线取舍

以下为本项目几何推论与实施约束：

1. **相邻接触界面共享同一空间曲面。** 两侧不能各自按法线施加同一标量噪声：`p+n*h` 和 `p-n*h` 是不同位置，会造成空隙或穿插。应由统一 patch/cutter 生成几何，两侧复用位置并反转绕序；多个切面交汇的边和角点同样要一致。
2. **边界零位移只能保持接缝，不能使该边界变锯齿。** 官方 Depth/Noise Ramp 的边缘衰减适合在保持原轮廓时增加内部起伏。[S6] 真正改变外轮廓，需要不规则切割面与两侧原外表面重新求交，或在破坏发生后局部替换闭合边带。
3. **不要为整个 Chunk 只准备一个统一破损版本。** 一个邻块删除后，其余接缝仍应完整；局部几何必须按受影响 patch 组合，并对相邻洞口合并/角点做确定处理。
4. **明确区分 reference physics 与 render detail。** 当前 `surfaces.py` 对显示网格仍检查凸性、体积和质心；`pipeline.py` 也拒绝未登记编辑/Modifier。不能简单插入 bevel/noise 后绕过检查。若保留原凸代理和质量，显示细节须使用原参考质心坐标，建立单独封闭性/自交/薄壁/最大偏差验证，并声明碰撞近似；若细节足以影响玩法，则同步重算物理/连接，而非继续宣称完全相同。
5. **微细节与轮廓细节分配不同尺度。** 首轮可试厘米级稀疏 chip 配合毫米级材质纹理，限制在局部厚度允许的范围；这是试调方向，不是已批准的通用振幅。细部增加后，仍要保留大尺度块面，而不是变成均匀高频毛刺。

## 建议交付顺序与验收

| 阶段 | 内容 | 主要依赖 | 验收重点 |
| --- | --- | --- | --- |
| A：先让动作成立 | 分层时序、材料/规模事件、异形共享碎屑、烟尘序列、反馈预算 | 现有 ECS；少量新视觉素材 | 单击及连击都能读到冲击→碎裂→消散，破口能及时看清 |
| B：融合环境 | Depth fade、近相机淡出、少量落地反弹、方向光；选择性加入涡动 | WESL、深度纹理；有限碰撞预算 | 贴墙不硬切、近看不穿帮、碎屑不持续穿楼板；测 CPU/GPU 增量 |
| C：破口边带 | 真实暴露区域/边界缓存、材质边带及少量局部几何 | patch/rim 语义及逐实例状态 | 关闭 VFX 后仍有边缘层次，完整建筑无提前露缝 |
| D：高质量轮廓 | 共享切割面、闭合细节几何、独立显示/代理验证 | 离线烘焙与格式扩展 | 近景斜视、逆光、薄墙、角点均成立，不靠烟尘遮错 |

先在一个带门窗的混凝土墙角做 A/B，保留现有无特效/旧效果基线、固定种子和同一相机路径。除截图外录制短回放：0.05 / 0.15 / 0.4 / 1 / 3s 的分阶段画面和连续运动才能检验重量感。

几何验收应关闭 VFX，覆盖正视、掠射、背光、内外两侧；检查完整状态、不同删块顺序到达同一最终状态、连续邻块删除、多个建筑实例、门窗通行和薄壁不刺穿。使用中近远三个视距检查边缘 LOD 和细纹闪烁。

性能验收记录实际分辨率、优化构建、CPU 更新/生成时间、GPU 透明绘制/深度预通道增量、可见粒子/主碎屑/边带三角形/材质实例数及连续点击峰值。当前只验证过数量有上限，不据此承诺 60fps，也不预设 GPU 粒子一定更快。[S3、S11]

## 已完成验证与未决问题

- 已完成：读取当前 Rust/Blender 实现、三个导出清单统计、下列官方资料与固定版本源码核验；完成文档链接和差异检查。
- 未完成：新烟尘素材制作、深度淡出与 Hanabi 在本仓库的编译/GPU 集成、边带或几何原型、观感 A/B、性能实测。本文不构成这些功能已实现的证明。
- 待实施时确定：写实或偏风格化的目标；视觉主碎屑与完整被删质量的表现比例；资产格式扩展的编码方式；不同材料的外观预设；主碎屑是否需要少量临时碰撞。

## 参考资料（2026-11-17 核验）

- [S1：DICE / SIGGRAPH 2010 — Destruction Masking in Frostbite](https://advances.realtimerendering.com/s2010/Kihl-Destruction%20in%20Frostbite%28SIGGRAPH%202010%20Advanced%20RealTime%20Rendering%20Course%29.pdf)，PDF 第 3–6 页的破口几何/材质/碎屑组合。借鉴方法，不采用其旧平台参数。
- [S2：SideFX — Debris Source](https://www.sidefx.com/docs/houdini/nodes/sop/debrissource.html)，内部表面发射、分离状态及体积/深度/年龄属性。
- [S3：NVIDIA GPU Gems 3 — High-Speed, Off-Screen Particles](https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-23-high-speed-screen-particles)，23.1 的 overdraw/fill-rate、23.4 的 soft particles。
- [S4：Unity — Realistic Smoke Lighting with 6-way Lighting](https://unity.com/blog/engine-platform/realistic-smoke-with-6-way-lighting-in-vfx-graph)，六向光照响应、两张 RGBA 贴图、平面近似的限制与 Blender 导出示例；未核验其导出脚本对当前 Blender 5.2 的兼容性。
- [S5：SideFX — RBD Material Fracture](https://www.sidefx.com/docs/houdini/nodes/sop/rbdmaterialfracture.html)，Concrete 的 Chipping / Edge Detail / Interior Detail。
- [S6：SideFX — Voronoi Fracture Control](https://www.sidefx.com/docs/houdini/dyno/voronoifracture.html)、[RBD Interior Detail](https://www.sidefx.com/docs/houdini/nodes/sop/rbdinteriordetail.html)，边缘噪声衰减、内面细分与独立代理。
- [S7：SideFX — Boolean Fracture](https://www.sidefx.com/docs/houdini/nodes/sop/booleanfracture.html)，内外表面、Exterior Seams 和切割面属性。
- [S8：Bridson 等 — Curl-noise for Procedural Fluid Flow，SIGGRAPH 2007](https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph2007-curlnoise.pdf)，可控无散速度场与边界处理。
- [S9：Bevy v0.20.0 — Prepass 示例](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/shader/shader_prepass.rs)、[View Transformations WESL](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_pbr/src/render/view_transformations.wesl)，另比对本机锁定 0.20.1 子 crate 的同名实现。
- [S10：Hanabi v0.20.0 — README](https://github.com/djeedai/bevy_hanabi/blob/v0.20.0/README.md)、[Cargo.toml](https://github.com/djeedai/bevy_hanabi/blob/v0.20.0/Cargo.toml)、[Render Modifiers](https://github.com/djeedai/bevy_hanabi/blob/v0.20.0/src/modifier/output.rs)，依赖兼容表、碰撞功能表和 flipbook 等源码。
- [S11：Epic — Measuring Performance in Niagara](https://dev.epicgames.com/documentation/en-us/unreal-engine/measuring-performance-in-niagara)，在代表性场景中分开测量生成、模拟和渲染成本；不照搬 Unreal 的性能数字或集成接口。
