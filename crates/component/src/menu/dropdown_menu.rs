use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Bounds, Context, DismissEvent, Element, ElementId, Entity,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, Pixels, RenderOnce, SharedString, Styled, Window, prelude::FluentBuilder,
};

use crate::{Selectable, button::Button, menu::PopupMenu, popover::Popover};

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + IntoElement + 'static> {
    id: ElementId,
    anchor: Anchor,
    trigger: T,
    builder: Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
    /// The inset of the trigger's text, for a menu opening over the trigger.
    over_trigger: Option<Pixels>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            anchor: anchor.into(),
            trigger,
            builder: Rc::new(builder),
            on_open_change: None,
            over_trigger: None,
        }
    }

    /// Set the anchor corner for the dropdown menu popover.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Add a callback to be called when the menu opens or closes.
    ///
    /// The `&bool` parameter is the **new open state**.
    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }

    /// Opens the menu over the trigger as a macOS pop-up button does, with the
    /// label of its checked item, or else its first, on the trigger's text
    /// `text_inset` in from the trigger's left edge.
    ///
    /// The menu grows to the window's height before it scrolls, unless
    /// [`PopupMenu::max_h`] caps it.
    pub fn over_trigger(mut self, text_inset: impl Into<Pixels>) -> Self {
        self.over_trigger = Some(text_inset.into());
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        TriggerFocus::new(
            self.id.clone(),
            move |trigger_focus, trigger_bounds, window, cx| {
                self.render_popover(trigger_focus, trigger_bounds, window, cx)
                    .into_any_element()
            },
        )
    }
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render_popover(
        self,
        trigger_focus: FocusHandle,
        trigger_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Popover {
        let builder = self.builder.clone();
        // A trigger not drawn yet has no bounds to open over, so its menu opens
        // below it.
        let over_trigger = self.over_trigger.filter(|_| !trigger_bounds.is_empty());
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        Popover::new(SharedString::from(format!("popover:{}", self.id)))
            .appearance(false)
            .overlay_closable(false)
            .trigger(self.trigger)
            .anchor(self.anchor)
            .when_some(self.on_open_change, |this, callback| {
                this.on_open_change(move |open, window, cx| callback(open, window, cx))
            })
            .content(move |_, window, cx| {
                // Here is special logic to only create the PopupMenu once and reuse it.
                // Because this `content` will called in every time render, so we need to store the menu
                // in state to avoid recreating at every render.
                //
                // And we also need to rebuild the menu when it is dismissed, to rebuild menu items
                // dynamically for support `dropdown_menu` method, so we listen for DismissEvent below.
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        menu.update(cx, |menu, cx| {
                            menu.set_trigger_focus(Some(trigger_focus.clone()), cx);
                            if let Some(text_inset) = over_trigger {
                                menu.open_over_trigger(trigger_bounds, text_inset);
                            }
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        //
                        // Hold Weak handles here, not strong clones: the listener
                        // lives as long as `menu`, which `menu_state` owns, so a
                        // strong capture would close the cycle
                        // `menu_state -> menu -> listener -> menu_state` and leak
                        // the `PopupMenu` and `DropdownMenuState` when the trigger
                        // stops being rendered while the menu is open.
                        let popover_state = cx.entity().downgrade();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.downgrade();
                                move |_, _: &DismissEvent, window, cx| {
                                    if let Some(popover_state) = popover_state.upgrade() {
                                        popover_state.update(cx, |state, cx| {
                                            state.dismiss(window, cx);
                                        });
                                    }
                                    _ = menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                };

                menu.clone()
            })
    }
}

type TriggerFocusBuild =
    Box<dyn FnOnce(FocusHandle, Bounds<Pixels>, &mut Window, &mut App) -> AnyElement>;

/// Registers a focus handle on the trigger's dispatch node without ever
/// focusing it, so the menu opened from the trigger can resolve its shortcut
/// hints against the trigger's key contexts on the frame it opens. GPUI looks
/// a handle up in the previously rendered frame; the trigger was in it when
/// the menu was not yet. It also hands on the trigger's bounds as last drawn,
/// for a menu opening over the trigger.
struct TriggerFocus {
    id: ElementId,
    build: Option<TriggerFocusBuild>,
}

#[derive(Default)]
struct TriggerFocusState {
    focus_handle: Option<FocusHandle>,
    bounds: Bounds<Pixels>,
}

struct TriggerFocusFrame {
    focus_handle: FocusHandle,
    child: AnyElement,
}

impl TriggerFocus {
    fn new(
        id: ElementId,
        build: impl FnOnce(FocusHandle, Bounds<Pixels>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id,
            build: Some(Box::new(build)),
        }
    }
}

impl IntoElement for TriggerFocus {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TriggerFocus {
    type RequestLayoutState = TriggerFocusFrame;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (focus_handle, bounds) =
            window.with_optional_element_state::<TriggerFocusState, _>(id, |state, _| {
                let mut state = state.flatten().unwrap_or_default();
                let focus_handle = state
                    .focus_handle
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                ((focus_handle, state.bounds), Some(state))
            });
        let build = self.build.take().expect("TriggerFocus is laid out once");
        let mut child = build(focus_handle.clone(), bounds, window, cx);
        let layout_id = child.request_layout(window, cx);

        (
            layout_id,
            TriggerFocusFrame {
                focus_handle,
                child,
            },
        )
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_optional_element_state::<TriggerFocusState, _>(id, |state, _| {
            let mut state = state.flatten().unwrap_or_default();
            state.bounds = bounds;
            ((), Some(state))
        });
        window.set_focus_handle(&frame.focus_handle, cx);
        frame.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        frame: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        frame.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        KeyBinding, MouseButton, ParentElement as _, Render, TestAppContext, WeakEntity, actions,
        div, point, px,
    };
    use std::cell::{Cell, RefCell};

    actions!(dropdown_menu_test, [CopyText]);

    const CONTEXT: &str = "dropdown_menu_test";

    /// The story shape: the key binding lives in the key context of the
    /// trigger's ancestor, the menu names no `action_context`, and other
    /// content outside that context paints after the trigger.
    struct TestRoot {
        frames: Rc<Cell<usize>>,
    }

    impl Render for TestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.frames.set(self.frames.get() + 1);
            div()
                .size_full()
                .child(
                    div()
                        .key_context(CONTEXT)
                        .on_action(|_: &CopyText, _, _| {})
                        .child(
                            Button::new("trigger")
                                .label("Edit")
                                .w(px(100.))
                                .h(px(30.))
                                .dropdown_menu(|menu, _, _| menu.menu("Copy", Box::new(CopyText))),
                        ),
                )
                .child(div().child("Status"))
        }
    }

    /// Records the `PopupMenu` the dropdown builds, so a test can observe
    /// its release through a weak handle.
    struct MenuProbeRoot {
        menu: Rc<RefCell<Option<WeakEntity<PopupMenu>>>>,
    }

    impl Render for MenuProbeRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let slot = self.menu.clone();
            div().size_full().child(
                Button::new("trigger")
                    .label("Edit")
                    .w(px(100.))
                    .h(px(30.))
                    .dropdown_menu(move |menu, _, cx| {
                        *slot.borrow_mut() = Some(cx.weak_entity());
                        menu.menu("Copy", Box::new(CopyText))
                    }),
            )
        }
    }

    /// Opening the menu and closing the window without dismissing it must
    /// release the `PopupMenu`: the dismiss subscription lives as long as the
    /// menu, which `DropdownMenuState` owns, so strong captures in it formed
    /// a cycle that no element-state collection could break. The state still
    /// holds the menu while it is open, so the menu's release also proves the
    /// `DropdownMenuState` was released.
    ///
    /// The popover must go too: it holds the deferred-popover registration
    /// while open, and a leaked one keeps every later right-click menu
    /// stepping aside as if a popup were still showing.
    #[gpui::test]
    fn open_without_dismiss_releases_the_menu(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init(cx));
        let menu = Rc::new(RefCell::new(None::<WeakEntity<PopupMenu>>));

        {
            let (_, cx) = cx.add_window_view(|_, _| MenuProbeRoot { menu: menu.clone() });
            cx.update(|window, cx| window.draw(cx).clear(cx));

            // Click the trigger; the menu opens and is left open.
            cx.simulate_mouse_down(
                point(px(10.), px(10.)),
                MouseButton::Left,
                Default::default(),
            );
            cx.run_until_parked();
            assert!(
                menu.borrow()
                    .as_ref()
                    .and_then(|menu| menu.upgrade())
                    .is_some(),
                "the menu must be open before the window closes"
            );

            // Close the window without dismissing the menu.
            cx.update(|window, _| window.remove_window());
            cx.run_until_parked();
        }

        assert!(
            menu.borrow()
                .as_ref()
                .and_then(|menu| menu.upgrade())
                .is_none(),
            "the PopupMenu must be released with the window"
        );
        cx.update(|cx| {
            assert!(
                !gpui_base::GlobalState::is_in_deferred_context(cx),
                "the popover's deferred registration must be released with the window"
            )
        });
    }

    #[gpui::test]
    fn shortcut_hint_is_painted_on_the_frame_the_menu_opens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-c", CopyText, Some(CONTEXT))]);
        });
        let frames = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let frames = frames.clone();
            move |_, _| TestRoot { frames }
        });
        // The popup host captures its trigger bounds on the first frame.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let frames_before_open = frames.get();

        cx.simulate_mouse_down(
            point(px(10.), px(10.)),
            MouseButton::Left,
            Default::default(),
        );

        assert_eq!(
            frames.get(),
            frames_before_open + 1,
            "the press must be followed by exactly one frame for this to test the first one"
        );
        assert!(
            cx.debug_bounds("kbd:ctrl-c").is_some(),
            "the shortcut hint must be painted on the same frame as its item"
        );
    }

    /// Twelve options opening over their trigger, with option 6 checked.
    #[cfg(feature = "test-support")]
    struct OverTriggerRoot;

    #[cfg(feature = "test-support")]
    impl Render for OverTriggerRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().pt(px(200.)).pl(px(200.)).child(
                Button::new("trigger")
                    .label("Option 6")
                    .w(px(100.))
                    .h(px(30.))
                    .dropdown_menu(|menu, _, _| {
                        (0..12).fold(menu, |menu, ix| {
                            menu.menu_with_check(
                                format!("Option {ix}"),
                                ix == 6,
                                Box::new(CopyText),
                            )
                        })
                    })
                    .over_trigger(px(8.)),
            )
        }
    }

    #[cfg(feature = "test-support")]
    fn setup_over_trigger(cx: &mut TestAppContext) -> &mut gpui::VisualTestContext {
        cx.update(|cx| crate::init(cx));
        let (_, cx) = cx.add_window_view(|_, _| OverTriggerRoot);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx
    }

    #[cfg(feature = "test-support")]
    fn open_over_trigger(cx: &mut gpui::VisualTestContext) {
        cx.simulate_click(point(px(250.), px(215.)), Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(is_menu_shown(cx), "the menu opens over its trigger");
    }

    #[cfg(feature = "test-support")]
    fn is_menu_shown(cx: &mut gpui::VisualTestContext) -> bool {
        cx.update(|window, _| {
            gpui_base::test_support::snapshots(window)
                .iter()
                .any(|element| element.role() == Some(gpui::Role::Menu) && element.visible())
        })
    }

    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn a_menu_over_its_trigger_closes_when_its_window_scrolls_or_resizes(cx: &mut TestAppContext) {
        let cx = setup_over_trigger(cx);

        open_over_trigger(cx);
        cx.simulate_event(gpui::ScrollEvent {
            position: point(px(250.), px(215.)),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(is_menu_shown(cx), "a scroll over the menu scrolls the menu");
        cx.simulate_event(gpui::ScrollEvent {
            position: point(px(20.), px(20.)),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(!is_menu_shown(cx), "a scroll of the window closes the menu");

        open_over_trigger(cx);
        cx.simulate_resize(gpui::size(px(700.), px(500.)));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(!is_menu_shown(cx), "a resize of the window closes the menu");
    }

    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn a_menu_over_its_trigger_neither_shows_nor_takes_the_pointer_while_measured(
        cx: &mut TestAppContext,
    ) {
        let cx = setup_over_trigger(cx);

        cx.simulate_mouse_down(
            point(px(250.), px(215.)),
            MouseButton::Left,
            Default::default(),
        );
        let first = cx.update(|window, _| {
            let snapshots = gpui_base::test_support::snapshots(window);
            assert!(
                snapshots.iter().all(|element| !element.visible()
                    || !matches!(
                        element.role(),
                        Some(gpui::Role::Menu | gpui::Role::MenuItem)
                    )),
                "the menu and its items show only once placed"
            );
            snapshots
                .into_iter()
                .find(|element| element.label() == Some("Option 0"))
                .map(|element| element.bounds())
        });
        let first = first.expect("the menu lays out its items while measuring them");
        cx.simulate_mouse_move(first.center(), None, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(is_menu_shown(cx));
        let is_first_selected = cx.update(|window, _| {
            gpui_base::test_support::snapshots(window)
                .iter()
                .any(|element| {
                    element.label() == Some("Option 0") && element.selected() == Some(true)
                })
        });
        assert!(
            !is_first_selected,
            "the pointer over the measured menu reached an item"
        );
    }
}
