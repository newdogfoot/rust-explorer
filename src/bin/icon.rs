//! Reproducible vector master for the Finder and Dock icon. Coordinates are in a 1024 pt square.
#![allow(deprecated)]
use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::*;

fn point(x: f64, y: f64) -> NSPoint {
    NSPoint::new(x, y)
}

fn color(hex: u32) -> Retained<NSColor> {
    let channel = |shift| f64::from(((hex >> shift) & 255u32) as u8) / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.)
}

fn paint(shape: &NSBezierPath, bottom: u32, top: u32, edge: u32, width: f64) {
    let gradient = NSGradient::initWithStartingColor_endingColor(
        NSGradient::alloc(),
        &color(bottom),
        &color(top),
    )
    .expect("sRGB gradient");
    gradient.drawInBezierPath_angle(shape, 85.);
    color(edge).setStroke();
    shape.setLineWidth(width);
    shape.stroke();
}

fn folder_back() -> Retained<NSBezierPath> {
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(point(140., 174.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(68., 248.),
        point(90., 174.),
        point(68., 201.),
    );
    path.lineToPoint(point(68., 754.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(133., 820.),
        point(68., 798.),
        point(91., 820.),
    );
    path.lineToPoint(point(329., 820.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(380., 799.),
        point(353., 820.),
        point(366., 813.),
    );
    path.lineToPoint(point(435., 742.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(474., 727.),
        point(447., 730.),
        point(459., 727.),
    );
    path.lineToPoint(point(827., 727.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(881., 672.),
        point(862., 727.),
        point(881., 707.),
    );
    path.lineToPoint(point(881., 244.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(810., 174.),
        point(881., 196.),
        point(854., 174.),
    );
    path.closePath();
    path
}

fn folder_front() -> Retained<NSBezierPath> {
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(point(142., 174.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(84., 212.),
        point(116., 174.),
        point(94., 186.),
    );
    path.curveToPoint_controlPoint1_controlPoint2(
        point(104., 267.),
        point(94., 222.),
        point(99., 241.),
    );
    path.lineToPoint(point(158., 538.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(219., 589.),
        point(165., 572.),
        point(181., 589.),
    );
    path.lineToPoint(point(476., 589.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(520., 607.),
        point(495., 589.),
        point(508., 595.),
    );
    path.lineToPoint(point(549., 634.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(591., 650.),
        point(562., 646.),
        point(574., 650.),
    );
    path.lineToPoint(point(910., 650.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(954., 592.),
        point(943., 650.),
        point(962., 628.),
    );
    path.lineToPoint(point(880., 239.));
    path.curveToPoint_controlPoint1_controlPoint2(
        point(802., 174.),
        point(870., 192.),
        point(846., 174.),
    );
    path.closePath();
    path
}

fn main() {
    let mtm = MainThreadMarker::new().expect("icon renderer runs on the main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(1024., 1024.));
    image.lockFocus();

    // The high left tab and slanted front make the silhouette legible at 16 px.
    paint(&folder_back(), 0xa95b0d, 0xffbc3b, 0xb6741c, 7.);
    let paper = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        NSRect::new(point(132., 332.), NSSize::new(696., 368.)),
        28.,
        28.,
    );
    paint(&paper, 0xdce8ef, 0xfffdf3, 0xc1a879, 5.);
    let divider = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        NSRect::new(point(148., 316.), NSSize::new(710., 337.)),
        31.,
        31.,
    );
    paint(&divider, 0x176aa8, 0x388fbc, 0x266c93, 5.);
    paint(&folder_front(), 0xf5a817, 0xffdf72, 0xb5791c, 7.);

    image.unlockFocus();
    let rep = NSBitmapImageRep::imageRepWithData(&image.TIFFRepresentation().unwrap()).unwrap();
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .unwrap();
    std::fs::write("resources/AppIcon.png", png.to_vec()).unwrap();
}
