//! 面板注册表（#119 批①）——Q-IDE `app/dock.rs:95-121`
//! `register_dock_panels` 模式：布局恢复（批②）按 persistent_name
//! 重建实体；本批交付注册/重建契约与测试，宿主接线归批②。

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{AnyView, App, Global};

/// 面板构造器：批② 布局恢复时按名重建（返回任意视图化实体，宿主
/// downcast 取回具体类型——蓝本 zed 经 `PanelHandle::to_any` 同通道）。
pub type PanelBuilder = Arc<dyn Fn(&mut App) -> AnyView>;

/// persistent_name → 构造器。gpui 全局（蓝本 Q-IDE 在 CodeWindow 内
/// 持 map；本批提为全局注册表——跨窗口共享，与 gpui-base
/// `registry.rs:59` `PanelRegistry`（Global）同构）。
#[derive(Default)]
pub struct PanelRegistry {
    builders: HashMap<&'static str, PanelBuilder>,
}

impl Global for PanelRegistry {}

impl PanelRegistry {
    /// 取全局（缺席则装默认空表——幂等）。
    pub fn default_global(cx: &mut App) {
        if cx.try_global::<Self>().is_none() {
            cx.set_global(Self::default());
        }
    }

    /// 注册（同名覆写——最新构造器胜出；布局恢复取最后注册者）。
    pub fn register(&mut self, name: &'static str, builder: PanelBuilder) {
        self.builders.insert(name, builder);
    }

    /// 全局注册便捷面。
    pub fn register_global(name: &'static str, builder: PanelBuilder, cx: &mut App) {
        Self::default_global(cx);
        cx.global_mut::<Self>().register(name, builder);
    }

    /// 按名重建（未知名 = None——坏布局静默跳过的裁决入口，批② 消费）。
    pub fn build(&self, name: &str, cx: &mut App) -> Option<AnyView> {
        self.builders.get(name).map(|build| build(cx))
    }

    /// 全局按名重建便捷面。构造器先克隆出注册表借用域再执行——
    /// 构造器内部经 `App::new` 需要对 `cx` 的可变访问。
    pub fn build_global(name: &str, cx: &mut App) -> Option<AnyView> {
        let builder = cx.try_global::<Self>()?.builders.get(name)?.clone();
        Some(builder(cx))
    }

    /// 已注册名集（批② 恢复面遍历 + 测试唯一性断言用）。
    pub fn names(&self) -> Vec<&'static str> {
        self.builders.keys().copied().collect()
    }
}
