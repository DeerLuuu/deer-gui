//! Vulkan 后端的**真实环境**验证。
//!
//! 这些测试跑在本机真实驱动上（不是 mock）：
//! - 若本机没有 Vulkan loader/驱动，会**优雅跳过**（`eprintln` + 返回），
//!   而不是失败 —— 因为 CI 可能真的没有 GPU；
//! - 若 loader 在，则**必须**通过 —— 这才是「符号声明与结构体布局正确」的证据。
//!
//! 用 `cargo test -p deer-vk -- --nocapture` 可以看到枚举到的显卡。

use deer_gpu::Backend;
use deer_vk::VkBackend;

#[test]
fn vk_backend_enumerates_real_adapters() {
    match VkBackend::new() {
        Ok(vk) => {
            let adapters = vk.adapters();
            println!("Vulkan 后端名: {}", vk.name());
            println!("枚举到 {} 个物理设备：", adapters.len());
            for (i, a) in adapters.iter().enumerate() {
                println!("  [{i}] {} | {:?} | {}", a.name, a.kind, a.driver);
            }

            assert!(!adapters.is_empty(), "VkBackend 构造成功 ⇒ 必须至少有一个设备");
            // 设备名不能是空/乱码 —— 若结构体布局错了，这里会读到垃圾，
            // 而这是最容易被忽略、也最能暴露布局错误的一处。
            for a in &adapters {
                assert!(
                    !a.name.is_empty(),
                    "设备名为空 ⇒ device_name 的 offset 或布局错了"
                );
                assert!(
                    a.name.is_ascii() || a.name.chars().all(|c| !c.is_control()),
                    "设备名含控制字符 ⇒ 布局错了：{:?}",
                    a.name
                );
                assert!(
                    !a.driver.is_empty(),
                    "驱动串为空 ⇒ apiVersion/driverVersion 读错了"
                );
            }
        }
        Err(e) => {
            // 无 Vulkan 环境：明确跳过，不伪装成通过
            println!("跳过：本机没有可用的 Vulkan（{e}）");
        }
    }
}

#[test]
fn vk_backend_opens_a_real_device_and_reports_m2b_boundaries() {
    // M1 阶段这条断言是「open() 必须明确报未实现」。**M2b 之后它变成了真设备**，
    // 所以这里改成验证新契约：拿到真设备，且**尚未实现的部分明确报错**（不许假装能用）。
    let Ok(vk) = VkBackend::new() else {
        println!("跳过：本机没有可用的 Vulkan");
        return;
    };

    let mut device = vk.open(0).expect("M2b 之后 open(0) 必须返回设备");
    assert!(
        !device.info().name.is_empty(),
        "适配器名不能为空（说明真的枚举到了设备）"
    );

    // 诚实边界①：还没建交换链就 begin_frame ⇒ 明确报错，而不是给一个空帧
    let err = device
        .begin_frame()
        .err()
        .expect("没有交换链时 begin_frame 必须报错");
    assert!(
        err.to_string().contains("交换链"),
        "错误信息要说清原因，实际：{err}"
    );

    // 诚实边界②：纹理（字形图集上传）是 M3 ⇒ 明确报错
    let err = device
        .create_texture(deer_gpu::TextureDesc {
            width: 4,
            height: 4,
            format: deer_gpu::TargetFormat::Rgba8Unorm,
            readable: false,
        })
        .expect_err("纹理创建未实现，必须报错");
    assert!(err.to_string().contains("M3"), "实际：{err}");

    // 越界的适配器索引也必须报错（而不是 panic）
    assert!(vk.open(999).is_err(), "越界索引必须返回错误");
}
