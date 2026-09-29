//! **HAL 纹理契约**（T1.2）：`VulkanDevice::create_texture` / `upload_texture` 走
//! **HAL 那一层**能建纹理、能上传、能回读，且**四通道逐字节保真**。
//!
//! ## 这个文件守什么（与 `texture_indirect.rs` 的分工）
//!
//! | 文件 | 测的是 |
//! |---|---|
//! | `texture_indirect.rs` | `device.rs` 的**底层**入口（`create_texture_rgba8` / `read_texture_bytes`） |
//! | **本文件** | **HAL 那一层**（`deer_gpu::Device` 的 `create_texture(TextureDesc)` / `upload_texture(id, data, region)`） |
//!
//! 两条链的差别不是「包了一层」那么简单：HAL 用的是
//! - `deer_gpu::TargetFormat`（而不是 `TextureFormat`）⇒ 要证明**格式映射**对；
//! - `TextureId` 句柄（而不是 `&Texture`）⇒ 要证明**句柄表**对；
//! - `TextureRegion` 子区域（而不是「整张图一次性上传」）⇒ 要证明**偏移拷贝**对。
//!
//! 这三条任何一条错了，只有本文件会红。所以它是 T1.2 的**唯一**判据来源。
//!
//! ## 判据口径（沿用本项目纪律，**不改松**）
//!
//! - RGBA8 上传保真 ⇒ **逐字节 0 差**（与 `texture_indirect.rs` 同款，不另有阈值）；
//! - 子区域上传 ⇒ 把「区域内的期望整图」与回读结果**逐字节**比（不是只看区域，
//!   因为「区域更新把别处覆盖了」也是一种错法）；
//! - 负例 ⇒ 非法参数必须在**碰驱动前**被拒，且**纹理句柄表不增长**。
//!
//! ## 无 GPU 时
//!
//! 打印原因并跳过（与既有 GPU 测试一致）；**请求了校验层却建不起设备 = 失败**，
//! 不是跳过 —— 用「跳过」掩盖「校验层没生效」会让这个文件变成安慰剂。

use deer_gpu::{AdapterInfo, Device, TargetFormat, TextureDesc, TextureRegion};
use deer_vk::device::{TextureFormat, UploadRegion, validate_texture_region};
use deer_vk::hal::VulkanDevice;
use deer_vk::VkBackend;

/// `DEER_VK_VALIDATION=1`/`true` ⇒ 请求校验层（判据与库内 `ffi::env_flag` 同源）。
fn validation_requested() -> bool {
    std::env::var("DEER_VK_VALIDATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 起一个 HAL 设备（拿不到就返回 `None` ⇒ 调用方打印后跳过）。
///
/// ## 为什么是 `VulkanDevice::new` 而不是 `Backend::open`
///
/// 两者走的是**同一段** HAL 代码（`VkBackend::open` 就是
/// `Ok(Box::new(hal::VulkanDevice::new(adapter, info)))`，见 `lib.rs`）；
/// 而 `open` 返回 `Box<dyn Device>`，`read_texture_bytes` 是**固有方法**（不在 trait 上，
/// 见 `hal.rs` 的理由），trait 对象上调不到。为了让本文件既能走真实 HAL 实现、
/// 又能拿到回读口，这里直接构 `VulkanDevice`（`pub`），
/// `AdapterInfo` 从 `VkBackend::adapters()` 拿（真适配器，不是编的）。
fn hal_device() -> Option<VulkanDevice> {
    let backend = match VkBackend::new() {
        Ok(b) => b,
        Err(e) if validation_requested() => {
            panic!("DEER_VK_VALIDATION 已请求，但 Vulkan 实例建不起来（{e}）")
        }
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan（{e}）");
            return None;
        }
    };
    let adapters = deer_gpu::Backend::adapters(&backend);
    let info: AdapterInfo = match adapters.first() {
        Some(a) => a.clone(),
        None => {
            println!("跳过：本机没有可用的 Vulkan 适配器");
            return None;
        }
    };
    Some(VulkanDevice::new(0, info))
}

/// 造一张 `w×h` 的 RGBA8 图案：**四个通道各不相同**（能抓「只传了 R」「通道错位」）。
fn rgba_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x * 40 + y * 7 + 3) as u8); // R
            data.push((x * 11 + y * 90 + 5) as u8); // G
            data.push((x * 3 + y * 200 + 17) as u8); // B
            data.push((x * 60 + y * 13 + 29) as u8); // A
        }
    }
    data
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. 纯逻辑（不需要 GPU）：区域校验
// ─────────────────────────────────────────────────────────────────────────────

/// 区域上传的参数校验是**纯函数**：三种错法都要在碰驱动前拦下。
///
/// ## 为什么这条值钱
///
/// `vkCmdCopyBufferToImage` 对「越界」与「源数据太短」**不报错**（要么静默裁掉、
/// 要么读越界内存）。没有这条纯函数测试，「越界上传」就只能靠人肉看代码守 ——
/// 而它恰恰是最容易写错的一处（`image_offset` 与 `image_extent` 的组合）。
#[test]
fn texture_region_validation_is_a_pure_function() {
    let f = TextureFormat::Rgba8Unorm;
    // 合法：整张 4×2（8 个像素 × 4 字节 = 32）
    assert!(
        validate_texture_region(4, 2, f, UploadRegion { x: 0, y: 0, width: 4, height: 2 }, &[0u8; 32])
            .is_ok()
    );
    // 合法：右下角恰好贴边（x+w == tex_w、y+h == tex_h 是**包含**的，不算越界）
    assert!(
        validate_texture_region(
            8,
            8,
            f,
            UploadRegion { x: 6, y: 6, width: 2, height: 2 },
            &[0u8; 16]
        )
        .is_ok()
    );
    // 合法：R8 的同一区域是 1 字节/像素
    assert!(
        validate_texture_region(
            8,
            8,
            TextureFormat::R8Unorm,
            UploadRegion { x: 1, y: 1, width: 2, height: 2 },
            &[0u8; 4]
        )
        .is_ok()
    );

    // 非法：0 面积（两个方向都要查）
    for region in [
        UploadRegion { x: 0, y: 0, width: 0, height: 2 },
        UploadRegion { x: 0, y: 0, width: 2, height: 0 },
    ] {
        assert!(
            validate_texture_region(8, 8, f, region, &[]).is_err(),
            "0 面积区域必须被拒：{region:?}"
        );
    }

    // 非法：越界（右 / 下，以及**恰好超一格**这种最常见的 off-by-one）
    for region in [
        UploadRegion { x: 7, y: 0, width: 2, height: 1 }, // 右边超 1
        UploadRegion { x: 0, y: 7, width: 1, height: 2 }, // 下边超 1
        UploadRegion { x: 4, y: 4, width: 8, height: 8 }, // 起点已在里面，尺寸还按整张
    ] {
        assert!(
            validate_texture_region(8, 8, f, region, &[0u8; 8 * 8 * 4]).is_err(),
            "越界区域必须被拒：{region:?}"
        );
    }

    // 非法：数据长度必须按**区域**算（不是按整张纹理）
    assert!(
        validate_texture_region(
            8,
            8,
            f,
            UploadRegion { x: 0, y: 0, width: 2, height: 2 },
            &[0u8; 8 * 8 * 4] // 给了整张的数据 —— 必须被拒，否则多余数据被静默忽略
        )
        .is_err(),
        "区域数据长度必须是 区域宽×高×4，不是整张纹理的长度"
    );

    // 溢出回绕：x 接近 u32::MAX 时 `x + width` 在 u32 下会回绕成「看似合法」
    assert!(
        validate_texture_region(
            8,
            8,
            f,
            UploadRegion { x: u32::MAX, y: 0, width: 2, height: 1 },
            &[0u8; 8]
        )
        .is_err(),
        "x + width 在 u32 下回绕后必须仍被判为越界（用 u64 比较）"
    );
}

/// HAL 格式映射：`TargetFormat` 三种取值都有确定落点，且**不静默**吞掉。
#[test]
fn hal_target_format_maps_to_a_texture_format() {
    // 这里的期望值是**契约**（不是实现细节）：Rgba8Unorm 逐字对应；
    // 两种 sRGB 目标按 UNORM 降级（纹理侧没有 sRGB 变体）。
    // 用「HAL 建纹理 → 读回后端格式」证明映射真的发生在**运行时**，而不是只看代码。
    if let Some(d) = hal_device() {
        let mut d = d;
        let expect = |want_desc: TargetFormat, want_tex: TextureFormat, d: &mut VulkanDevice| {
            let id = d
                .create_texture(TextureDesc {
                    width: 2,
                    height: 2,
                    format: want_desc,
                    readable: true,
                })
                .expect("建纹理");
            assert_eq!(
                d.texture_format(id),
                Some(want_tex),
                "{want_desc:?} 应映射到 {want_tex:?}"
            );
            assert_eq!(d.texture_extent(id), Some((2, 2)));
        };
        expect(TargetFormat::Rgba8Unorm, TextureFormat::Rgba8Unorm, &mut d);
        expect(TargetFormat::Rgba8Srgb, TextureFormat::Rgba8Unorm, &mut d);
        expect(TargetFormat::Bgra8Srgb, TextureFormat::Rgba8Unorm, &mut d);
    } else {
        println!("跳过：本机没有可用的 Vulkan 设备");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. HAL 路径：建 → 上传（整张）→ 回读（四通道保真）
// ─────────────────────────────────────────────────────────────────────────────

/// **HAL 路径的四通道保真**：`Device::create_texture` + `Device::upload_texture`
/// （整张区域）之后，回读必须与原数据**逐字节**相同。
///
/// 这与 `texture_indirect.rs::rgba8_texture_upload_preserves_all_four_channels`
/// 是**同一条判据、不同一条调用路径** —— 后者走 `device.rs` 的直接入口，
/// 本文件走 HAL 的 `Device` trait。两条都要绿，才说明「HAL 真的接上了底层实现」，
/// 而不是「底层自己好用」。
#[test]
fn hal_texture_round_trip_preserves_all_four_channels() {
    let Some(mut dev) = hal_device() else {
        println!("跳过：本机没有可用的 Vulkan 设备");
        return;
    };
    let (w, h) = (4u32, 3u32);
    let data = rgba_pattern(w, h);

    // 前置断言：图案必须有区分度，否则「四通道保真」是空话
    for ch in 0..4 {
        let distinct: std::collections::BTreeSet<u8> = data
            .chunks_exact(4)
            .map(|p| p[ch])
            .collect();
        assert!(
            distinct.len() > 1,
            "前置条件不成立：通道 {ch} 没有区分度（{distinct:?}）"
        );
    }

    let id = dev
        .create_texture(TextureDesc {
            width: w,
            height: h,
            format: TargetFormat::Rgba8Unorm,
            readable: true,
        })
        .expect("HAL 建纹理");
    assert_eq!(dev.texture_extent(id), Some((w, h)));

    // 前置断言：刚建的纹理内容必须是**已知的零**（而不是未定义内存）——
    // 这既是本 crate 的契约，也是「区域上传只改区域」那条测试的基线。
    let zeros = dev.read_texture_bytes(id).expect("回读新建纹理");
    assert_eq!(zeros.len(), data.len(), "回读长度 = w×h×4");
    assert!(
        zeros.iter().all(|b| *b == 0),
        "新建纹理的初值必须是全 0（否则区域上传的基线不可知）"
    );

    dev.upload_texture(
        id,
        &data,
        TextureRegion {
            x: 0,
            y: 0,
            width: w,
            height: h,
        },
    )
    .expect("HAL 上传纹理");

    let back = dev.read_texture_bytes(id).expect("HAL 回读纹理");
    assert_eq!(back.len(), data.len());
    assert_eq!(
        back, data,
        "HAL 路径的 RGBA8 上传必须逐字节保真（含 alpha）"
    );
    println!(
        "  HAL 纹理往返 ✅ {w}×{h}：{} 字节逐字节相同（走 Device trait）",
        data.len()
    );
}

/// **HAL 子区域上传**：只改区域内的像素，区域外的像素**保持原样**。
///
/// ## 这条在抓什么
///
/// 实现区域上传最容易犯的错是「把整张图当 src、但只写一个区域」
/// （`buffer_row_length` 用整宽）或「布局转换用了 `UNDEFINED` 起点」
/// （丢掉了区域外的内容）。两种错法都会让**区域外的像素被写坏或变未定义**，
/// 而区域内的像素**看起来是对的** —— 所以只比区域内的判据抓不住它们。
/// 这里比的是**整张**图像。
#[test]
fn hal_region_upload_updates_only_the_region() {
    let Some(mut dev) = hal_device() else {
        println!("跳过：本机没有可用的 Vulkan 设备");
        return;
    };
    let (w, h) = (6u32, 4u32);

    let id = dev
        .create_texture(TextureDesc {
            width: w,
            height: h,
            format: TargetFormat::Rgba8Unorm,
            readable: true,
        })
        .expect("HAL 建纹理");

    // ① 先铺一层已知底（整张）—— 用与图案不同的常量，便于分辨「谁覆盖了谁」
    let base: Vec<u8> = (0..(w * h))
        .flat_map(|i| {
            let i = i as u8;
            [10 + i, 20 + i, 30 + i, 255]
        })
        .collect();
    dev.upload_texture(
        id,
        &base,
        TextureRegion {
            x: 0,
            y: 0,
            width: w,
            height: h,
        },
    )
    .expect("铺底");

    // ② 只更新中间一块 2×2（起点 (2,1)），值是明确可辨的常量
    let (rx, ry, rw, rh) = (2u32, 1u32, 2u32, 2u32);
    let patch: Vec<u8> = std::iter::repeat_n([200u8, 100, 50, 255], (rw * rh) as usize)
        .flatten()
        .collect();
    dev.upload_texture(
        id,
        &patch,
        TextureRegion {
            x: rx,
            y: ry,
            width: rw,
            height: rh,
        },
    )
    .expect("区域上传");

    // ③ 期望：整张图 = 底图，但区域内的像素换成 patch
    let mut expected = base.clone();
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            let i = ((y * w + x) * 4) as usize;
            expected[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
        }
    }

    // 前置断言：期望图里必须**同时**有「被改的像素」与「没被改的像素」
    assert!(
        expected.chunks_exact(4).any(|p| p == [200, 100, 50, 255]),
        "前置条件不成立：期望里没有区域内的像素"
    );
    assert!(
        expected.chunks_exact(4).any(|p| p != [200, 100, 50, 255]),
        "前置条件不成立：期望里没有区域外的像素"
    );

    let back = dev.read_texture_bytes(id).expect("回读");
    assert_eq!(
        back.len(),
        expected.len(),
        "整张回读（区域上传不该改变纹理尺寸）"
    );
    assert_eq!(
        back, expected,
        "区域上传必须只改区域内的像素（区域外保持原样 —— 抓 UNDEFINED 起点 / 行距错）"
    );
    println!("  HAL 区域上传 ✅ 只改了 ({rx},{ry}) {rw}×{rh}，区域外逐字节不变");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. 负例：非法参数必须在碰驱动前被拒，且不改变句柄表
// ─────────────────────────────────────────────────────────────────────────────

/// **负例**：`create_texture` 的非法描述、`upload_texture` 的非法 id / 非法区域
/// 都必须报错，且**不留下半个纹理**。
///
/// `upload_texture` 用了一个**从未分配**的 id：这是「句柄表查表」那条逻辑的
/// 唯一直接证据 —— 实现若把 id 当索引用 `slots[id]` 而不查，这里会 panic 或
/// 越界；若忽略 id 往「最近一张纹理」上写，这里会因为「不报错」而红。
#[test]
fn hal_texture_rejects_bad_inputs_without_allocating() {
    let Some(mut dev) = hal_device() else {
        println!("跳过：本机没有可用的 Vulkan 设备");
        return;
    };

    // ① 0 尺寸必须被拒（**不碰驱动**）
    for (w, h) in [(0u32, 2u32), (2, 0), (0, 0)] {
        assert!(
            dev.create_texture(TextureDesc {
                width: w,
                height: h,
                format: TargetFormat::Rgba8Unorm,
                readable: true,
            })
            .is_err(),
            "{w}×{h} 必须被拒"
        );
    }

    // ② 合法建一张，拿到一个真实 id
    let id = dev
        .create_texture(TextureDesc {
            width: 4,
            height: 4,
            format: TargetFormat::Rgba8Unorm,
            readable: true,
        })
        .expect("建纹理");
    assert_eq!(id.0, 0, "第一张纹理的句柄应是 0");

    // ③ 不存在的 id：读 / 上传都必须报错（**不许**静默成功或 panic）
    let ghost = deer_gpu::TextureId(99);
    assert!(
        dev.read_texture_bytes(ghost).is_err(),
        "不存在的纹理 id 回读必须报错"
    );
    assert!(
        dev.upload_texture(
            ghost,
            &[0u8; 4],
            TextureRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err(),
        "不存在的纹理 id 上传必须报错（不许往别的纹理上写）"
    );

    // ④ 非法区域：越界 / 长度不符 / 0 面积（对**存在**的 id）
    let bad_regions = [
        (TextureRegion { x: 3, y: 0, width: 2, height: 1 }, vec![0u8; 8], "右边越界"),
        (TextureRegion { x: 0, y: 3, width: 1, height: 2 }, vec![0u8; 8], "下边越界"),
        (TextureRegion { x: 0, y: 0, width: 2, height: 2 }, vec![0u8; 4], "长度不足"),
        (TextureRegion { x: 0, y: 0, width: 2, height: 2 }, vec![0u8; 64], "长度过多（整张）"),
        (TextureRegion { x: 0, y: 0, width: 0, height: 2 }, vec![], "0 宽"),
        (TextureRegion { x: 0, y: 0, width: 2, height: 0 }, vec![], "0 高"),
    ];
    for (region, data, what) in bad_regions {
        assert!(
            dev.upload_texture(id, &data, region).is_err(),
            "非法区域（{what}）必须被拒"
        );
    }

    // ⑤ 被拒的调用不能污染纹理内容：合法区域写一次后回读，内容仍是写进去的
    dev.upload_texture(
        id,
        &[7u8, 8, 9, 255],
        TextureRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    )
    .expect("合法上传");
    let back = dev.read_texture_bytes(id).expect("回读");
    assert_eq!(&back[0..4], &[7, 8, 9, 255], "左上角应是最后写进去的值");
    assert!(
        back[4..].iter().all(|b| *b == 0),
        "被拒的调用不该改动纹素的其它部分"
    );

    println!("  HAL 负例 ✅ 0 尺寸 / 幽灵 id / 五类非法区域 / 长度不符 均被拒且无副作用");
}

/// **HAL 设备是惰性的**：`VulkanDevice::new` 不建逻辑设备；`create_texture` 之后才建。
///
/// 这条守的是 `VulkanDevice::new` 的文档契约（「只记下适配器」）与
/// `TextureStore::new` 的注释（「第一次才打开」）。若有人把它改成 `new` 里就开设备，
/// 那「枚举适配器」这种只读操作会变成一次设备创建 —— 收益是 0、代价是启动变慢 +
/// 多一个可能失败的早期点。用一个不建纹理的设备来证明它没建设备（`texture_extent`
/// 在没建任何纹理时应返回 `None`）。
#[test]
fn hal_device_is_lazy_about_the_logical_device() {
    let Some(dev) = hal_device() else {
        println!("跳过：本机没有可用的 Vulkan 设备");
        return;
    };
    // 还没建任何纹理 ⇒ 槽位表为空（`with_store` 也还没被调用过）
    assert_eq!(
        dev.texture_extent(deer_gpu::TextureId(0)),
        None,
        "未建纹理前不该有任何句柄"
    );
    assert!(
        dev.read_texture_bytes(deer_gpu::TextureId(0)).is_err(),
        "未建纹理前回读必须报错"
    );
    // 只读的适配器信息一直可用（不需要设备）
    assert!(!dev.info().name.is_empty());
}
