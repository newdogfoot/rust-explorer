//! Native vector artwork and row painting shared by the explorer and settings.
#![allow(unsafe_op_in_unsafe_fn)]
use super::widgets::rect;
use crate::core::{
    model::{Entry, Kind},
    theme::{Color, IconStyle, Layout, Resolved},
};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, NSObjectProtocol},
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::cell::Cell;

// Theme changes are immediate, not a sequence of implicit layer animations.
pub struct PaintTransaction;
impl PaintTransaction {
    pub fn new() -> Self {
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        Self
    }
}
impl Drop for PaintTransaction {
    fn drop(&mut self) {
        objc2_quartz_core::CATransaction::commit();
    }
}

pub fn color(c: Color) -> Retained<NSColor> {
    let [r, g, b] = c.rgb();
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.)
}
pub fn fill(r: NSRect, c: Color, radius: f64) {
    color(c).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, radius, radius).fill();
}
fn shape(
    points: &[(f64, f64)],
    closed: bool,
    fill_color: Option<Color>,
    stroke: Color,
    width: f64,
) {
    let path = NSBezierPath::bezierPath();
    for (i, (x, y)) in points.iter().enumerate() {
        let point = NSPoint::new(*x, 64. - y);
        if i == 0 {
            path.moveToPoint(point)
        } else {
            path.lineToPoint(point)
        }
    }
    if closed {
        path.closePath();
    }
    if let Some(c) = fill_color {
        color(c).setFill();
        path.fill();
    }
    color(stroke).setStroke();
    path.setLineWidth(width);
    path.stroke();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    Folder,
    Pdf,
    Sheet,
    Image,
    Archive,
    Text,
    Audio,
    Video,
    Code,
    Presentation,
    Package,
    Link,
    File,
}
impl Glyph {
    pub fn for_entry(e: &Entry) -> Self {
        match e.kind {
            Kind::Folder => Self::Folder,
            Kind::Package => Self::Package,
            Kind::Symlink => Self::Link,
            Kind::File => match e
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "pdf" => Self::Pdf,
                "csv" | "tsv" | "xls" | "xlsx" | "numbers" | "ods" => Self::Sheet,
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "tiff" | "tif" | "bmp"
                | "svg" | "avif" => Self::Image,
                "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "zst" => Self::Archive,
                "txt" | "md" | "rtf" | "doc" | "docx" | "pages" | "odt" => Self::Text,
                "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" => Self::Audio,
                "mp4" | "mov" | "mkv" | "avi" | "webm" => Self::Video,
                "ppt" | "pptx" | "key" | "odp" => Self::Presentation,
                "rs" | "py" | "js" | "ts" | "html" | "css" | "json" | "toml" | "sh" | "c"
                | "cpp" | "swift" => Self::Code,
                _ => Self::File,
            },
        }
    }
    fn hue(self) -> Color {
        Color(match self {
            Self::Pdf => 0xc54f45,
            Self::Sheet => 0x268462,
            Self::Image => 0x5f77c2,
            Self::Archive => 0xac7d31,
            Self::Audio => 0x9360b1,
            Self::Video => 0xb4557c,
            Self::Code => 0x4879a8,
            Self::Presentation => 0xc47736,
            _ => 0x5a7c92,
        })
    }
}
pub fn icon(p: Resolved, g: Glyph) -> Retained<NSImage> {
    let block = block2::RcBlock::new(move |_: NSRect| {
        draw_icon(p, g);
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(64., 64.), false, &block);
    image.setTemplate(false);
    image
}
fn draw_icon(p: Resolved, g: Glyph) {
    let style = p.spec.icon;
    let c = if g == Glyph::Folder {
        p.folder
    } else if style == IconStyle::Mono {
        p.muted
    } else {
        g.hue()
    };
    let mut edge = c.mix(Color(0x17222d), 0.28);
    if style == IconStyle::Outline {
        let toward = Color(if p.spec.dark { 0xffffff } else { 0x000000 });
        for _ in 0..24 {
            if edge.contrast(p.canvas) >= 4.6 && edge.contrast(p.selection) >= 4.6 {
                break;
            }
            edge = edge.mix(toward, 0.14);
        }
    }
    let back = c.mix(Color(0x17222d), 0.2);
    let paper = Color(if p.spec.dark { 0xe4eaf1 } else { 0xffffff });
    let paper_edge = Color(0xa9b7c5);
    if g == Glyph::Folder {
        let outline = style == IconStyle::Outline;
        shape(
            &[
                (6., 17.),
                (6., 14.),
                (25., 14.),
                (31., 20.),
                (58., 20.),
                (58., 51.),
                (6., 51.),
            ],
            true,
            Some(if outline { p.canvas.mix(c, 0.12) } else { back }),
            edge,
            if outline { 3. } else { 1.5 },
        );
        if style == IconStyle::Pocket {
            shape(
                &[(15., 21.), (44., 21.), (44., 45.), (15., 45.)],
                true,
                Some(paper),
                paper,
                1.,
            );
            shape(
                &[
                    (5., 29.),
                    (25., 29.),
                    (30., 24.),
                    (59., 24.),
                    (55., 51.),
                    (8., 51.),
                ],
                true,
                Some(c),
                edge,
                1.5,
            );
        } else if outline {
            shape(&[(7., 27.), (57., 27.)], false, None, edge, 2.5);
        } else {
            shape(
                &[(5., 24.), (59., 24.), (56., 52.), (8., 52.)],
                true,
                Some(c),
                edge,
                1.5,
            );
            if matches!(style, IconStyle::Flat | IconStyle::Badge) {
                shape(&[(13., 34.), (27., 34.)], false, None, edge, 2.5);
            } else {
                shape(
                    &[(8., 26.), (54., 26.)],
                    false,
                    None,
                    c.mix(Color(0xffffff), 0.3),
                    2.,
                );
            }
        }
        return;
    }
    let outline = style == IconStyle::Outline;
    let badge = style == IconStyle::Badge;
    let border = if outline { edge } else { paper_edge };
    shape(
        &[(13., 6.), (39., 6.), (51., 18.), (51., 57.), (13., 57.)],
        true,
        Some(if outline {
            p.canvas.mix(c, 0.045)
        } else if badge {
            paper.mix(c, 0.13)
        } else {
            paper
        }),
        border,
        if outline { 2.6 } else { 1.5 },
    );
    shape(
        &[(39., 6.), (39., 19.), (51., 19.)],
        false,
        None,
        border,
        1.5,
    );
    if matches!(style, IconStyle::Classic | IconStyle::Duo | IconStyle::Mono) {
        fill(rect(14., 8., 7., 49.), c, 0.);
    }
    if badge {
        fill(rect(17., 14., 30., 25.), c, 4.);
    }
    let mark = if badge {
        if c.contrast(Color(0xffffff)) >= 4.5 {
            Color(0xffffff)
        } else {
            Color(0x000000)
        }
    } else {
        edge
    };
    let line = |pts: &[(f64, f64)]| shape(pts, false, None, mark, 2.6);
    match g {
        Glyph::Pdf => {
            line(&[
                (26., 45.),
                (26., 29.),
                (33., 29.),
                (36., 32.),
                (36., 36.),
                (33., 39.),
                (26., 39.),
            ]);
        }
        Glyph::Sheet => {
            shape(
                &[(24., 29.), (42., 29.), (42., 46.), (24., 46.)],
                true,
                None,
                mark,
                2.,
            );
            line(&[(24., 35.), (42., 35.)]);
            line(&[(24., 40.), (42., 40.)]);
            line(&[(30., 29.), (30., 46.)]);
            line(&[(36., 29.), (36., 46.)]);
        }
        Glyph::Image => {
            shape(
                &[(23., 45.), (30., 35.), (35., 41.), (39., 36.), (44., 45.)],
                true,
                Some(mark),
                mark,
                1.,
            );
            fill(rect(36., 31., 6., 6.), mark, 3.);
        }
        Glyph::Archive => line(&[
            (30., 25.),
            (30., 28.),
            (35., 28.),
            (35., 31.),
            (30., 31.),
            (30., 34.),
            (35., 34.),
            (35., 37.),
            (30., 37.),
            (30., 40.),
            (35., 40.),
            (35., 45.),
            (29., 45.),
            (29., 40.),
        ]),
        Glyph::Audio => {
            line(&[(28., 43.), (28., 28.), (40., 26.), (40., 41.)]);
            fill(rect(22., 17., 7., 5.), mark, 2.);
            fill(rect(34., 19., 7., 5.), mark, 2.);
        }
        Glyph::Video => shape(
            &[(26., 28.), (43., 37.), (26., 46.)],
            true,
            Some(mark),
            mark,
            1.,
        ),
        Glyph::Code => {
            line(&[(29., 29.), (23., 37.), (29., 45.)]);
            line(&[(37., 29.), (43., 37.), (37., 45.)]);
        }
        Glyph::Presentation => {
            shape(
                &[(23., 28.), (43., 28.), (43., 42.), (23., 42.)],
                true,
                None,
                mark,
                2.,
            );
            line(&[(33., 42.), (33., 48.)]);
            line(&[(27., 48.), (39., 48.)]);
        }
        Glyph::Package => {
            shape(
                &[(24., 30.), (42., 30.), (42., 46.), (24., 46.)],
                true,
                None,
                mark,
                2.,
            );
            line(&[(24., 30.), (33., 24.), (42., 30.)]);
            line(&[(33., 30.), (33., 38.)]);
        }
        Glyph::Link => {
            line(&[(24., 45.), (24., 34.), (42., 34.)]);
            line(&[(36., 28.), (42., 34.), (36., 40.)]);
        }
        _ => {
            line(&[(25., 30.), (41., 30.)]);
            line(&[(25., 36.), (41., 36.)]);
            line(&[(25., 42.), (37., 42.)]);
        }
    }
}

#[derive(Default)]
pub struct RowPaint {
    palette: Cell<Resolved>,
    sidebar: Cell<bool>,
}
define_class!(
 #[unsafe(super=NSTableRowView)]
 #[name="REThemedRowView"]
 #[thread_kind=MainThreadOnly]
 #[ivars=RowPaint]
 pub struct ThemeRow;
 unsafe impl NSObjectProtocol for ThemeRow {}
 impl ThemeRow {
  #[unsafe(method(interiorBackgroundStyle))]
  fn background_style(&self)->NSBackgroundStyle {NSBackgroundStyle::Normal}
  #[unsafe(method(drawBackgroundInRect:))]
  fn background(&self,_:NSRect){unsafe{let p=self.ivars().palette.get();let r=self.bounds();let side=self.ivars().sidebar.get();fill(r,if side{p.rail}else{p.canvas},0.);
    if !side && matches!(p.spec.layout,Layout::Lined|Layout::Compact){fill(rect(0.,r.size.height-1.,r.size.width,1.),p.line,0.);}
    if !side && p.spec.layout==Layout::Cards {fill(rect(4.,2.,r.size.width-8.,r.size.height-4.),p.canvas.mix(p.shell,0.35),p.radius());}
  }}
  #[unsafe(method(drawSelectionInRect:))]
  fn selection(&self,_:NSRect){unsafe{let p=self.ivars().palette.get();let r=self.bounds();let side=self.ivars().sidebar.get();fill(rect(3.,1.,r.size.width-6.,r.size.height-2.),if side{p.side_selection}else{p.selection},p.radius());if p.spec.layout==Layout::Edge{fill(rect(3.,1.,3.,r.size.height-2.),p.accent,1.);}}}
 }
);
pub unsafe fn row(
    mtm: MainThreadMarker,
    table: &NSTableView,
    p: Resolved,
    sidebar: bool,
) -> Retained<NSTableRowView> {
    let id = ns_string!("theme-row");
    let view = table
        .makeViewWithIdentifier_owner(id, None)
        .and_then(|v| v.downcast::<ThemeRow>().ok())
        .unwrap_or_else(|| {
            let obj = ThemeRow::alloc(mtm).set_ivars(Default::default());
            let view: Retained<ThemeRow> = msg_send![super(obj),initWithFrame:rect(0.,0.,300.,30.)];
            view.setIdentifier(Some(id));
            view
        });
    view.ivars().palette.set(p);
    view.ivars().sidebar.set(sidebar);
    view.setNeedsDisplay(true);
    Retained::into_super(view)
}

pub unsafe fn paint_item(item: &NSCollectionViewItem, p: Resolved) {
    if let Some(layer) = item.view().layer() {
        layer.setCornerRadius(p.radius());
        layer.setBorderWidth(if item.isSelected() { 1. } else { 0. });
        layer.setBorderColor(Some(&color(p.accent).CGColor()));
        layer.setBackgroundColor(Some(
            &if item.isSelected() {
                color(p.selection)
            } else if p.spec.layout == Layout::Cards {
                color(p.canvas.mix(p.shell, 0.35))
            } else {
                NSColor::clearColor()
            }
            .CGColor(),
        ));
    }
    if let Some(label) = item.textField() {
        label.setTextColor(Some(&color(p.ink)));
    }
}

pub unsafe fn restyle_row(row: &NSTableRowView, p: Resolved, sidebar: bool) {
    if let Some(row) = row.downcast_ref::<ThemeRow>() {
        row.ivars().palette.set(p);
        row.ivars().sidebar.set(sidebar);
        row.setNeedsDisplay(true);
    }
}

#[cfg(feature = "native-ui-tests")]
pub fn row_palette(row: &NSTableRowView) -> Option<Resolved> {
    row.downcast_ref::<ThemeRow>()
        .map(|v| v.ivars().palette.get())
}
