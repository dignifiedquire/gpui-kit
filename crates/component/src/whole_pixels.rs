//! Lengths and placement on whole pixels, so that 1 px lines cover one row.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Rems, Window,
};

/// Returns `length` at the window's rem size, rounded to whole pixels.
pub(crate) fn to_whole_pixels(length: Rems, window: &Window) -> Pixels {
    length.to_pixels(window.rem_size()).round()
}

/// An element placing its child on whole pixels, wherever its parent puts it.
///
/// A popup may land between pixels, as a menu at the pointer or a dialog
/// centred in an odd space does, and a 1 px line there renders as two rows at
/// half contrast.
pub(crate) struct WholePixelOrigin {
    child: AnyElement,
}

impl WholePixelOrigin {
    /// Returns `child` placed on whole pixels.
    pub(crate) fn new(child: impl IntoElement) -> Self {
        Self {
            child: child.into_any_element(),
        }
    }
}

impl IntoElement for WholePixelOrigin {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WholePixelOrigin {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // GPUI adds the element offset snapped to device pixels, so the layout
        // origin is what remains once that snapped offset is taken away.
        let layout_origin = bounds.origin - window.pixel_snap_point(window.element_offset());
        let origin = bounds.origin.map(|edge| edge.round());
        window.with_absolute_element_offset(origin - layout_origin, |window| {
            self.child.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
