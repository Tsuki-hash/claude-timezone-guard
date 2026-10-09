fn main() {
    // 把 icon.ico 嵌进 exe（资源管理器图标 / 任务栏归属图标 / 安装包图标）。
    // 窗口运行时的任务栏图标由 main.rs 的 `with_icon` 单独设置
    // （同一张图的 RGBA，不设的话 eframe 会用它自带的 "e" 默认图标）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // manifest_optional：不存在清单时跳过 manifest 注入，但图标必须编进去
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
