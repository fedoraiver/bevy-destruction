# 自然不规则预破碎原型

这是隔离调研工具，不是正式建筑生成器的新模式。它保持墙体和窗洞不变，通过共享分区生成无明显周期的边界，控制碎块尺度，输出互相贴合的刚性碎块和精确复合凸代理。

需要 Blender 5.2.3 LTS。在仓库根目录运行，使用独立后台实例：

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' `
  --background --factory-startup --python-exit-code 1 `
  --python tools/blender/prototypes/curved_partition.py -- `
  --output output/irregular-fracture/natural `
  --seed 20261117 --target-size 1.4 --step 0.1
```

输出平面/不规则/拆开后的对照图、两个随机种子对照图、可打开的 `.blend`、SVG 及 `report.json`。`--no-render` 跳过渲染；`--target-size 1.2` 可观察更小、更多的碎块。`--step` 是边界离散精度，支持 0.05–0.2 m，并需要兼容墙和窗洞尺寸；它不是碎块尺度。

原型检查最终碎块大小及形状、真实邻接合并、闭合性、共享界面、体积/质心/完整惯性、凸代理和窗洞净空。不能满足条件时明确失败，不丢弃正面积碎片。相同种子与配置可在已验证工具版本下复现。

运行 7 项测试：

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' `
  --background --factory-startup --python-exit-code 1 `
  --python tools/blender/prototypes/test_curved_partition.py
```

隔离原型覆盖二维薄墙沿厚度挤出的分块，输出保存在独立目录。正式资产使用共享分区和 v2 多面片接口。详见 [v2 资产格式](../ASSET_FORMAT.md)。
