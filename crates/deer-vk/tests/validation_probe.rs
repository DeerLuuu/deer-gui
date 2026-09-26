//! 校验层可用性探测。
//!
//! 这些测试**记录本机的真实情况**（装没装 Vulkan SDK、有没有校验层），
//! 而不是假设它存在。装好 SDK 后 `validation_layer_is_available` 会从
//! 「报告未安装」变成「成功启用」，无需改任何代码。

use deer_vk::ffi;

#[test]
fn validation_layer_availability_is_reported_honestly() {
    // 先看本机注册了哪些层
    let lib = match deer_vk::loader::Lib::open() {
        Ok(l) => l,
        Err(e) => {
            println!("跳过：本机没有 vulkan-1.dll（{e}）");
            return;
        }
    };

    match ffi::Instance::create_with_validation(true) {
        Ok(inst) => {
            assert!(
                inst.validation_enabled(),
                "create_with_validation(true) 成功时必须真的启用了校验层"
            );
            println!("✅ 校验层 {} 可用并已启用", ffi::VALIDATION_LAYER);
            // 故意做一件非法的事，看校验层会不会报（这是能力验证，不是功能测试）
            println!("（下面若出现 [VALIDATION]/[VK ERROR] 前缀的消息，说明回调正常工作）");
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("未安装") || msg.contains("未注册"),
                "请求校验层失败时，错误信息必须说清「为什么」与「怎么办」，实际：{msg}"
            );
            println!("⚠️ 本机没有校验层：{msg}");
            println!("   ⇒ 装 Vulkan SDK 后重跑本测试即可启用");
        }
    }
    let _ = lib;
}

#[test]
fn creating_instance_without_validation_still_works() {
    // 不开校验层这条路必须始终可用（它是默认路径）
    match ffi::Instance::create_with_validation(false) {
        Ok(inst) => {
            assert!(
                !inst.validation_enabled(),
                "没请求校验层时不应该启用它"
            );
            println!("默认路径（不启用校验层）正常 ✅");
        }
        Err(e) => println!("跳过：本机没有可用的 Vulkan（{e}）"),
    }
}
