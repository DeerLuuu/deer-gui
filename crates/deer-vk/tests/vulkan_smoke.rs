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
fn vk_backend_reports_unsupported_for_device_creation_in_m1() {
    // M1 只到实例 + 枚举。`open()` 必须**明确报未实现**，而不是返回一个假装能用的设备。
    // 假成功比报错难查得多，所以这条断言是刻意的。
    let Ok(vk) = VkBackend::new() else {
        println!("跳过：本机没有可用的 Vulkan");
        return;
    };
    let err = match vk.open(0) {
        Ok(_) => panic!("M1 阶段 open() 必须返回错误，而不是返回一个假装能用的设备"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("M2"),
        "错误信息应指明实现里程碑，实际：{msg}"
    );

    // 越界的适配器索引也必须报错（而不是 panic）
    assert!(vk.open(999).is_err(), "越界索引必须返回错误");
}
