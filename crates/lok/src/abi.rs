//! LOK ABI 面：`#[repr(C)]` 类结构 + 逐成员 nSize/offset 守卫。
//!
//! ## 来源与实证（issue「固定依据」）
//!
//! - ABI 定义 = `include/LibreOfficeKit/LibreOfficeKit.h` @ pin commit
//!   `0d51fdc57aef6707466320821b8c2931c00297c0`（MPL-2.0；本项目按事实
//!   接口重写并注明出处，不整段复制头文件）。
//! - 布局实证（本机，2026-11-09）：Debian LibreOffice 25.2.3.2 的
//!   `LibreOfficeKitDocumentClass` 成员 0-18 与 pin 头完全同序（`paste`
//!   两版同在 #19）；`LibreOfficeKitClass` 成员 0-12 同序（25.2 在 #13
//!   多一个后遭移除的 `sendDialogEvent`，pin 头该位为 `runLoop` 之后
//!   直接 `setOption`——本 crate 绑定止于 `runLoop`#12，不受影响）。
//! - **防漂移纪律**：LO 26.x 曾在 Doc #19 之前插入成员（ElyOffice slot
//!   注释实证 vtable 中段插入事故）→ 本 crate 只绑定两版同序段
//!   （Doc 0-18 / Kit 0-12），每次调用前 [`abi::has_member`] 守卫，
//!   越界成员需要扩绑时必须先补实证。

use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulonglong};

/// 官方回调签名：`void (*)(int nType, const char* pPayload, void* pData)`
/// （LibreOfficeKitTypes.h @ pin commit）。回调在 LO 内核线程触发，
/// worker 侧只做入队转发（见 [`crate::worker`]）。
pub type LokCallback =
    unsafe extern "C" fn(n_type: c_int, payload: *const c_char, data: *mut c_void);

/// hook 签名：`libreofficekit_hook_2(const char* install_path, const char* user_profile_url)`
/// → `LibreOfficeKit*`。user_profile_url 必须是 `file:` URI（官方
/// LibreOfficeKitInit.h 明言传路径名会被拒）。
pub type LokHook2 = unsafe extern "C" fn(
    install_path: *const c_char,
    user_profile_url: *const c_char,
) -> *mut LokKit;

/// LibreOfficeKit 对象（句柄壳：单指针指向类结构）。
#[repr(C)]
pub struct LokKit {
    pub p_class: *const LokKitClass,
}

/// LibreOfficeKitDocument 对象（句柄壳）。
#[repr(C)]
pub struct LokDocument {
    pub p_class: *const LokDocClass,
}

/// 类结构：`size_t nSize` + 函数指针成员（仅 append 语义；本 crate
/// 绑定到 `runLoop`#12 为止）。
#[repr(C)]
pub struct LokKitClass {
    pub n_size: usize,
    pub destroy: Option<unsafe extern "C" fn(p_this: *mut LokKit)>,
    pub document_load:
        Option<unsafe extern "C" fn(p_this: *mut LokKit, p_url: *const c_char) -> *mut LokDocument>,
    pub get_error: Option<unsafe extern "C" fn(p_this: *mut LokKit) -> *mut c_char>,
    pub document_load_with_options: Option<
        unsafe extern "C" fn(
            p_this: *mut LokKit,
            p_url: *const c_char,
            p_options: *const c_char,
        ) -> *mut LokDocument,
    >,
    pub free_error: Option<unsafe extern "C" fn(p_free: *mut c_char)>,
    pub register_callback: Option<
        unsafe extern "C" fn(p_this: *mut LokKit, p_callback: LokCallback, p_data: *mut c_void),
    >,
    pub get_filter_types: Option<unsafe extern "C" fn(p_this: *mut LokKit) -> *mut c_char>,
    pub set_optional_features:
        Option<unsafe extern "C" fn(p_this: *mut LokKit, features: c_ulonglong)>,
    pub set_document_password: Option<
        unsafe extern "C" fn(p_this: *mut LokKit, p_url: *const c_char, p_password: *const c_char),
    >,
    pub get_version_info: Option<unsafe extern "C" fn(p_this: *mut LokKit) -> *mut c_char>,
    pub run_macro: Option<unsafe extern "C" fn(p_this: *mut LokKit, p_url: *const c_char) -> c_int>,
    #[allow(dead_code)]
    pub sign_document: Option<
        unsafe extern "C" fn(
            p_this: *mut LokKit,
            p_url: *const c_char,
            cert: *const u8,
            cert_len: c_int,
            key: *const u8,
            key_len: c_int,
        ) -> bool,
    >,
    #[allow(dead_code)]
    pub run_loop: Option<
        unsafe extern "C" fn(
            p_this: *mut LokKit,
            poll: Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>,
            wake: Option<unsafe extern "C" fn(*mut c_void)>,
            data: *mut c_void,
        ),
    >,
}

/// 文档类结构（绑定面止于 #40 `postWindowExtTextInputEvent`；#41 起
/// 不绑——需要时逐槽实证再扩）。
///
/// **#19-#40 扩绑实证（2026-11-11，LibreOffice 25.2.3.2 Debian 13
/// `libreofficekit-dev` 头）**：`libreofficekit-dev` 包头逐槽枚举
/// Doc 类共 79 个函数指针成员（#19 paste、#20 setGraphicSelection、
/// #22 getCommandValues、#23 setClientZoom、#24 setClientVisibleArea、
/// #27 setView、#40 postWindowExtTextInputEvent…），与运行时实测
/// `nSize=640`（= 8 字节 × (1 nSize + 79 成员)）吻合；与仓内既有实证
/// 「#19 位两版同在 paste」一致。#19-#39 声明为 dead_code（仅占位对
/// 齐布局，未调用零风险）；#40 为本批消费面（post_text IME）。
/// 注意：ely-lok 按自家补丁版 LO 绝对槽号绑定，不可照抄（比对报告
/// #115 §3）；本表槽号仅对 25.2.3.2 头负责，跨版本调用前必过
/// `has_member` 运行时守卫。
#[repr(C)]
pub struct LokDocClass {
    pub n_size: usize,
    pub destroy: Option<unsafe extern "C" fn(p_this: *mut LokDocument)>,
    pub save_as: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_url: *const c_char,
            p_format: *const c_char,
            p_filter_options: *const c_char,
        ) -> c_int,
    >,
    pub get_document_type: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    pub get_parts: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    pub get_part_page_rectangles:
        Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> *mut c_char>,
    #[allow(dead_code)]
    pub get_part: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    pub set_part: Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_part: c_int)>,
    #[allow(dead_code)]
    pub get_part_name:
        Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_part: c_int) -> *mut c_char>,
    #[allow(dead_code)]
    pub set_part_mode: Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_mode: c_int)>,
    pub paint_tile: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_buffer: *mut u8,
            canvas_width: c_int,
            canvas_height: c_int,
            tile_pos_x: c_int,
            tile_pos_y: c_int,
            tile_width: c_int,
            tile_height: c_int,
        ),
    >,
    #[allow(dead_code)]
    pub get_tile_mode: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    pub get_document_size: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, p_width: *mut c_long, p_height: *mut c_long),
    >,
    pub initialize_for_rendering:
        Option<unsafe extern "C" fn(p_this: *mut LokDocument, p_arguments: *const c_char)>,
    pub register_callback: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_callback: LokCallback,
            p_data: *mut c_void,
        ),
    >,
    pub post_key_event: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_type: c_int,
            n_char_code: c_int,
            n_key_code: c_int,
        ),
    >,
    pub post_mouse_event: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_type: c_int,
            n_x: c_int,
            n_y: c_int,
            n_count: c_int,
            n_buttons: c_int,
            n_modifier: c_int,
        ),
    >,
    pub post_uno_command: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_command: *const c_char,
            p_arguments: *const c_char,
            notify_when_finished: bool,
        ),
    >,
    #[allow(dead_code)]
    pub set_text_selection: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, n_type: c_int, n_x: c_int, n_y: c_int),
    >,
    pub get_text_selection: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_mime_type: *const c_char,
            p_used_mime_type: *mut *mut c_char,
        ) -> *mut c_char,
    >,
    /// #19 paste（25.2.3.2 头；dead_code 占位对齐）。
    #[allow(dead_code)]
    pub paste: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_mime_type: *const c_char,
            p_data: *const c_char,
            n_size: usize,
        ) -> bool,
    >,
    /// #20 setGraphicSelection（dead_code 占位）。
    #[allow(dead_code)]
    pub set_graphic_selection: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, n_type: c_int, n_x: c_int, n_y: c_int),
    >,
    /// #21 resetSelection（dead_code 占位）。
    #[allow(dead_code)]
    pub reset_selection: Option<unsafe extern "C" fn(p_this: *mut LokDocument)>,
    /// #22 getCommandValues（dead_code 占位）。
    #[allow(dead_code)]
    pub get_command_values: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, p_command: *const c_char) -> *mut c_char,
    >,
    /// #23 setClientZoom（dead_code 占位；表格面/视口协作批次启用）。
    #[allow(dead_code)]
    pub set_client_zoom: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_tile_pixel_width: c_int,
            n_tile_pixel_height: c_int,
            n_tile_twip_width: c_int,
            n_tile_twip_height: c_int,
        ),
    >,
    /// #24 setClientVisibleArea（dead_code 占位）。
    #[allow(dead_code)]
    pub set_client_visible_area: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_x: c_int,
            n_y: c_int,
            n_width: c_int,
            n_height: c_int,
        ),
    >,
    /// #25 createView（dead_code 占位）。
    #[allow(dead_code)]
    pub create_view: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    /// #26 destroyView（dead_code 占位）。
    #[allow(dead_code)]
    pub destroy_view: Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_id: c_int)>,
    /// #27 setView（dead_code 占位；多视图/Calc 激活面）。
    #[allow(dead_code)]
    pub set_view: Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_id: c_int)>,
    /// #28 getView（dead_code 占位）。
    #[allow(dead_code)]
    pub get_view: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    /// #29 getViewsCount（dead_code 占位）。
    #[allow(dead_code)]
    pub get_views_count: Option<unsafe extern "C" fn(p_this: *mut LokDocument) -> c_int>,
    /// #30 renderFont（dead_code 占位）。
    #[allow(dead_code)]
    pub render_font: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_font_name: *const c_char,
            p_char: *const c_char,
            p_font_width: *mut c_int,
            p_font_height: *mut c_int,
        ) -> *mut u8,
    >,
    /// #31 getPartHash（dead_code 占位）。
    #[allow(dead_code)]
    pub get_part_hash:
        Option<unsafe extern "C" fn(p_this: *mut LokDocument, n_part: c_int) -> *mut c_char>,
    /// #32 paintPartTile（dead_code 占位；演示面批次启用）。
    #[allow(dead_code)]
    pub paint_part_tile: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            p_buffer: *mut u8,
            n_part: c_int,
            n_mode: c_int,
            n_canvas_width: c_int,
            n_canvas_height: c_int,
            n_tile_pos_x: c_int,
            n_tile_pos_y: c_int,
            n_tile_width: c_int,
            n_tile_height: c_int,
        ),
    >,
    /// #33 getViewIds（dead_code 占位）。
    #[allow(dead_code)]
    pub get_view_ids: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, p_array: *mut c_int, n_size: usize) -> bool,
    >,
    /// #34 setOutlineState（dead_code 占位）。
    #[allow(dead_code)]
    pub set_outline_state: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            b_column: bool,
            n_level: c_int,
            n_index: c_int,
            b_hidden: bool,
        ),
    >,
    /// #35 paintWindow（dead_code 占位）。
    #[allow(dead_code)]
    pub paint_window: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_window_id: c_uint,
            p_buffer: *mut u8,
            x: c_int,
            y: c_int,
            width: c_int,
            height: c_int,
        ),
    >,
    /// #36 postWindow（dead_code 占位）。
    #[allow(dead_code)]
    pub post_window: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_window_id: c_uint,
            n_action: c_int,
            p_data: *const c_char,
        ),
    >,
    /// #37 postWindowKeyEvent（dead_code 占位）。
    #[allow(dead_code)]
    pub post_window_key_event: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_window_id: c_uint,
            n_type: c_int,
            n_char_code: c_int,
            n_key_code: c_int,
        ),
    >,
    /// #38 postWindowMouseEvent（dead_code 占位）。
    #[allow(dead_code)]
    pub post_window_mouse_event: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_window_id: c_uint,
            n_type: c_int,
            n_x: c_int,
            n_y: c_int,
            n_count: c_int,
            n_buttons: c_int,
            n_modifier: c_int,
        ),
    >,
    /// #39 setViewLanguage（dead_code 占位）。
    #[allow(dead_code)]
    pub set_view_language: Option<
        unsafe extern "C" fn(p_this: *mut LokDocument, n_id: c_int, language: *const c_char),
    >,
    /// #40 postWindowExtTextInputEvent（IME 组合/提交；本批消费面）。
    pub post_window_ext_text_input_event: Option<
        unsafe extern "C" fn(
            p_this: *mut LokDocument,
            n_window_id: c_uint,
            n_type: c_int,
            p_text: *const c_char,
        ),
    >,
}

/// `LIBREOFFICEKIT_HAS_MEMBER` 的 Rust 等价：`offsetof(member) + size
/// <= nSize`（官方宏只查 offsetof < nSize；对尾部成员补上自身大小更严）。
pub fn has_member(n_size: usize, offset: usize, size: usize) -> bool {
    offset.checked_add(size).is_some_and(|end| end <= n_size)
}

/// Kit 成员 offset 表（编译期常量，绑定面止于 run_loop）。
pub mod kit_offsets {
    use super::LokKitClass;
    macro_rules! off {
        ($name:ident, $field:ident) => {
            pub const $name: usize = std::mem::offset_of!(LokKitClass, $field);
        };
    }
    off!(DESTROY, destroy);
    off!(DOCUMENT_LOAD, document_load);
    off!(GET_ERROR, get_error);
    off!(DOCUMENT_LOAD_WITH_OPTIONS, document_load_with_options);
    off!(FREE_ERROR, free_error);
    off!(REGISTER_CALLBACK, register_callback);
    off!(SET_OPTIONAL_FEATURES, set_optional_features);
    off!(GET_VERSION_INFO, get_version_info);
    // B1：unipoll 主循环（worker unipoll_loop 进 runLoop；缺槽 = boot-failed）
    off!(RUN_LOOP, run_loop);
}

/// Doc 成员 offset 表（绑定面止于 get_text_selection#18）。
pub mod doc_offsets {
    use super::LokDocClass;
    macro_rules! off {
        ($name:ident, $field:ident) => {
            pub const $name: usize = std::mem::offset_of!(LokDocClass, $field);
        };
    }
    off!(DESTROY, destroy);
    off!(SAVE_AS, save_as);
    off!(GET_DOCUMENT_TYPE, get_document_type);
    off!(GET_PARTS, get_parts);
    off!(GET_PART_PAGE_RECTANGLES, get_part_page_rectangles);
    off!(SET_PART, set_part);
    off!(PAINT_TILE, paint_tile);
    off!(GET_DOCUMENT_SIZE, get_document_size);
    off!(INITIALIZE_FOR_RENDERING, initialize_for_rendering);
    off!(REGISTER_CALLBACK, register_callback);
    off!(POST_KEY_EVENT, post_key_event);
    off!(POST_MOUSE_EVENT, post_mouse_event);
    off!(POST_UNO_COMMAND, post_uno_command);
    off!(GET_TEXT_SELECTION, get_text_selection);
    off!(
        POST_WINDOW_EXT_TEXT_INPUT_EVENT,
        post_window_ext_text_input_event
    );
}

/// 文档类型（LibreOfficeKitEnums.h：TEXT=0 / SPREADSHEET=1 /
/// PRESENTATION=2 / DRAWING=3 / OTHER=4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Text,
    Spreadsheet,
    Presentation,
    Drawing,
    Other(c_int),
}

impl DocType {
    pub fn from_raw(v: c_int) -> DocType {
        match v {
            0 => DocType::Text,
            1 => DocType::Spreadsheet,
            2 => DocType::Presentation,
            3 => DocType::Drawing,
            other => DocType::Other(other),
        }
    }
}

/// 键事件类型（LOK_KEYEVENT_KEYINPUT=0 / KEYUP=1）。
pub const KEYEVENT_KEYINPUT: c_int = 0;
pub const KEYEVENT_KEYUP: c_int = 1;

/// 扩展文本输入（IME）事件类型（LibreOfficeKitEnums.h
/// LibreOfficeKitExtTextInputType，25.2.3.2 逐值实证：TEXTINPUT=0 /
/// TEXTINPUT_POS=1 / TEXTINPUT_END=2）。组合期发 0（预编辑串），
/// 提交发 2（定稿串）——ELYOffice ely-lok「LOK-07」同语义。
pub const EXT_TEXTINPUT: c_int = 0;
pub const EXT_TEXTINPUT_POS: c_int = 1;
pub const EXT_TEXTINPUT_END: c_int = 2;

/// postWindowExtTextInputEvent 的 window id：文档主窗口 = 0
/// （无弹窗对话框场景；#35+ 弹窗面启用前恒 0）。
pub const MAIN_WINDOW_ID: c_uint = 0;

/// 鼠标事件类型（LibreOfficeKitEnums.h 实证序：BUTTONDOWN=0 /
/// BUTTONUP=1 / MOVE=2——注意不是 down/up/move 的直觉序）。
pub const MOUSEEVENT_BUTTON_DOWN: c_int = 0;
pub const MOUSEEVENT_BUTTON_UP: c_int = 1;
pub const MOUSEEVENT_MOVE: c_int = 2;

/// 鼠标按键位（ElyOffice 同源：左=1）。
pub const MOUSE_BUTTON_LEFT: c_int = 1;

/// 可选特性位（LibreOfficeKitEnums.h：PART_IN_INVALIDATION=1<<2）。
pub const FEATURE_PART_IN_INVALIDATION_CALLBACK: c_ulonglong = 1 << 2;

/// 回调类型（值 = LibreOfficeKitEnums.h @ pin commit，与 25.2.3.2
/// 逐值 diff 实证一致；仅列本 crate 消费的子集）。
pub mod callbacks {
    /// tile 失效（需重绘）。
    pub const INVALIDATE_TILES: i32 = 0;
    /// 可见光标移动。
    pub const INVALIDATE_VISIBLE_CURSOR: i32 = 1;
    /// 文本选区变化。
    pub const TEXT_SELECTION: i32 = 2;
    /// 状态变更（payload 形如 ".uno:DocModified=v"）。
    pub const STATE_CHANGED: i32 = 8;
    /// 搜索无结果。
    pub const SEARCH_NOT_FOUND: i32 = 12;
    /// 文档尺寸变化。
    pub const DOCUMENT_SIZE_CHANGED: i32 = 13;
    /// 搜索命中（payload: searchResultSelection JSON）。
    pub const SEARCH_RESULT_SELECTION: i32 = 15;
    /// UNO 命令完成（payload: {"commandName","success"}）。
    pub const UNO_COMMAND_RESULT: i32 = 16;

    /// 回调类型名（诊断/事件帧用；未知值返回 None）。
    pub fn name(v: i32) -> Option<&'static str> {
        Some(match v {
            INVALIDATE_TILES => "INVALIDATE_TILES",
            INVALIDATE_VISIBLE_CURSOR => "INVALIDATE_VISIBLE_CURSOR",
            TEXT_SELECTION => "TEXT_SELECTION",
            STATE_CHANGED => "STATE_CHANGED",
            SEARCH_NOT_FOUND => "SEARCH_NOT_FOUND",
            DOCUMENT_SIZE_CHANGED => "DOCUMENT_SIZE_CHANGED",
            SEARCH_RESULT_SELECTION => "SEARCH_RESULT_SELECTION",
            UNO_COMMAND_RESULT => "UNO_COMMAND_RESULT",
            _ => return None,
        })
    }
}
