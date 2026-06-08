//! Chứa định nghĩa component `setting_item` - một block cài đặt có khả năng mở rộng (Expander) hoặc đứng độc lập.
//! Hỗ trợ animation khi mở rộng và căn chỉnh tự động với icon.

use windows_reactor::*;

/// Hàm tiện ích để hiển thị các biểu tượng (icon) chuẩn của Windows (Segoe Fluent Icons).
pub fn font_icon(character: char) -> TextBlock {
    text_block(character.to_string()).font_family("Segoe Fluent Icons")
}

/// Các thuộc tính cấu hình cho một mục cài đặt (`setting_item`)
#[derive(Clone, PartialEq)]
pub struct SettingItemProps {
    /// Biểu tượng của mục cài đặt (nếu có)
    pub icon: Option<char>,
    /// Tiêu đề chính
    pub title: String,
    /// Mô tả chi tiết phụ cho tiêu đề (hiển thị mờ và nhỏ hơn ở dưới)
    pub description: Option<String>,
    /// Element tương tác ở góc phải (VD: ToggleSwitch, Button)
    pub action: Option<Element>,
    /// Danh sách các mục con. Nếu có, thẻ cài đặt này sẽ biến thành dạng Expander (có thể xổ xuống)
    pub children: Option<Vec<SettingItemProps>>,
}

/// Nhận `action_element` từ ngoài vào để tránh việc phải clone toàn bộ `SettingItemProps`
fn render_inner_layout(props: &SettingItemProps, action_element: Element) -> Element {
    let description_el = props.description.as_ref().map_or(Element::Empty, |desc| {
        caption(desc.clone())
            .font_size(12.0)
            .foreground(ThemeRef::SecondaryText)
            .wrap()
            .into()
    });

    let content = vstack((body(props.title.clone()), description_el));

    // Gom nhóm các thuộc tính phụ thuộc vào icon để tính toán 1 lần
    let (icon_el, icon_col_width, content_margin_left) = match props.icon {
        Some(character) => (
            Into::<Element>::into(font_icon(character).font_size(18.0))
                .vertical_alignment(VerticalAlignment::Center)
                .horizontal_alignment(HorizontalAlignment::Center)
                .grid_column(0),
            GridLength::Pixel(54.0),
            0.0,
        ),
        None => (Element::Empty, GridLength::Pixel(0.0), 16.0),
    };

    grid((
        icon_el,
        content
            .margin(Thickness {
                left: content_margin_left,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            })
            .vertical_alignment(VerticalAlignment::Center)
            .grid_column(1),
        action_element
            .vertical_alignment(VerticalAlignment::Center)
            .margin(Thickness {
                left: 0.0,
                top: 0.0,
                right: 4.0,
                bottom: 0.0,
            })
            .grid_column(2),
    ))
    .columns([icon_col_width, GridLength::Star(1.0), GridLength::Auto])
    .min_height(64.0)
    .horizontal_alignment(HorizontalAlignment::Stretch)
    .into()
}

/// Element gốc cho một item cài đặt.
///
/// Nếu `props.children` tồn tại, component sẽ render một cấu trúc `Expander` tuỳ chỉnh
/// có khả năng xổ xuống (slide down animation) bằng cách kết hợp `ScrollViewer` và `LayoutAnimation`.
/// Nếu không, nó sẽ trả về một `Border` tĩnh chứa cấu hình.
pub fn setting_item(props: &SettingItemProps, cx: &mut RenderCx) -> Element {
    let (is_expanded, set_expanded) = cx.use_state(false);

    tracing::debug!(
        "DEBUG: setting_item rendered. title={}, is_expanded={}",
        props.title,
        is_expanded
    );

    let mut action_elements = Vec::new();
    if let Some(act) = &props.action {
        action_elements.push(act.clone());
    }

    if props.children.is_some() {
        let chevron_char = if is_expanded { "\u{E70E}" } else { "\u{E70D}" };
        let chevron_btn = button(chevron_char)
            .font_family("Segoe Fluent Icons")
            .font_size(14.0)
            .height(32.0)
            .width(32.0)
            .padding(0.0)
            .subtle()
            .on_click({
                let set_expanded = set_expanded.clone();
                move || set_expanded.call(!is_expanded)
            });

        action_elements.push(chevron_btn.into());
    }

    let final_action = match action_elements.len() {
        0 => Element::Empty,
        1 => action_elements.pop().unwrap(),
        _ => hstack(action_elements)
            .vertical_alignment(VerticalAlignment::Center)
            .horizontal_alignment(HorizontalAlignment::Right)
            .spacing(6.0)
            .into(),
    };

    let header_layout = render_inner_layout(props, final_action);
    let mut card_elements = vec![header_layout];

    if let Some(children) = &props.children {
        let separator_element = border(Element::Empty)
            .background(ThemeRef::CardStroke)
            .height(if is_expanded { 1.0 } else { 0.0 })
            .opacity(if is_expanded { 1.0 } else { 0.0 })
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .into();

        let mut children_list = Vec::new();
        for (i, child_prop) in children.iter().enumerate() {
            if i > 0 {
                children_list.push(
                    border(Element::Empty)
                        .height(1.0)
                        .background(ThemeRef::CardStroke)
                        .horizontal_alignment(HorizontalAlignment::Stretch)
                        .into(),
                );
            }

            let child_action = child_prop.action.clone().unwrap_or(Element::Empty);
            children_list.push(
                render_inner_layout(child_prop, child_action)
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .margin(Thickness {
                        left: 38.0,
                        right: 38.0,
                        top: 0.0,
                        bottom: 0.0,
                    })
                    .into(),
            );
        }

        let inner_border =
            border(vstack(children_list).horizontal_alignment(HorizontalAlignment::Stretch))
                .horizontal_alignment(HorizontalAlignment::Stretch);

        let children_container = scroll_viewer(inner_border)
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Hidden)
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .max_height(if is_expanded { f64::INFINITY } else { 0.0 })
            .opacity(if is_expanded { 1.0 } else { 0.0 })
            .with_layout_animation(LayoutAnimationConfig::spring().animate_size(true))
            .with_opacity_transition(std::time::Duration::from_millis(150))
            .into();

        card_elements.push(separator_element);
        card_elements.push(children_container);
    }

    border(vstack(card_elements).horizontal_alignment(HorizontalAlignment::Stretch))
        .corner_radius(4.0)
        .border_thickness(Thickness::from(1.0))
        .background(ThemeRef::CardBackground)
        .border_brush(ThemeRef::CardStroke)
        .with_layout_animation(LayoutAnimationConfig::spring().animate_size(true))
        .into()
}
