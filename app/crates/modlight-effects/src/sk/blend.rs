//! `SKBlendMode`: Skia's blend formulas on premultiplied floats (raster pipeline highp),
//! and the matching tiny-skia mode for canvas drawing.

use super::filters::F4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Clear,
    Src,
    Dst,
    SrcOver,
    DstOver,
    SrcIn,
    DstIn,
    SrcOut,
    DstOut,
    SrcATop,
    DstATop,
    Xor,
    Plus,
    Modulate,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Multiply,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    pub fn to_skia(self) -> tiny_skia::BlendMode {
        use tiny_skia::BlendMode as B;
        match self {
            Blend::Clear => B::Clear,
            Blend::Src => B::Source,
            Blend::Dst => B::Destination,
            Blend::SrcOver => B::SourceOver,
            Blend::DstOver => B::DestinationOver,
            Blend::SrcIn => B::SourceIn,
            Blend::DstIn => B::DestinationIn,
            Blend::SrcOut => B::SourceOut,
            Blend::DstOut => B::DestinationOut,
            Blend::SrcATop => B::SourceAtop,
            Blend::DstATop => B::DestinationAtop,
            Blend::Xor => B::Xor,
            Blend::Plus => B::Plus,
            Blend::Modulate => B::Modulate,
            Blend::Screen => B::Screen,
            Blend::Overlay => B::Overlay,
            Blend::Darken => B::Darken,
            Blend::Lighten => B::Lighten,
            Blend::ColorDodge => B::ColorDodge,
            Blend::ColorBurn => B::ColorBurn,
            Blend::HardLight => B::HardLight,
            Blend::SoftLight => B::SoftLight,
            Blend::Difference => B::Difference,
            Blend::Exclusion => B::Exclusion,
            Blend::Multiply => B::Multiply,
            Blend::Hue => B::Hue,
            Blend::Saturation => B::Saturation,
            Blend::Color => B::Color,
            Blend::Luminosity => B::Luminosity,
        }
    }

    /// `SkBlendMode_SupportsCoverageAsAlpha`: the modes for which scaling the source's
    /// alpha by coverage equals blending with that coverage.
    pub fn supports_coverage_as_alpha(self) -> bool {
        matches!(
            self,
            Blend::Dst
                | Blend::SrcOver
                | Blend::DstOver
                | Blend::DstOut
                | Blend::SrcATop
                | Blend::Xor
                | Blend::Plus
        )
    }

    /// Source `s` over destination `d`, both premultiplied.
    pub fn apply(self, s: F4, d: F4) -> F4 {
        let per = |f: &dyn Fn(f32, f32, f32, f32) -> f32| F4 {
            r: f(s.r, d.r, s.a, d.a),
            g: f(s.g, d.g, s.a, d.a),
            b: f(s.b, d.b, s.a, d.a),
            a: f(s.a, d.a, s.a, d.a),
        };
        let sep = |f: &dyn Fn(f32, f32, f32, f32) -> f32| F4 {
            r: f(s.r, d.r, s.a, d.a),
            g: f(s.g, d.g, s.a, d.a),
            b: f(s.b, d.b, s.a, d.a),
            a: s.a + d.a - s.a * d.a,
        };
        match self {
            Blend::Clear => F4::default(),
            Blend::Src => s,
            Blend::Dst => d,
            Blend::SrcOver => per(&|s, d, sa, _| s + d * (1.0 - sa)),
            Blend::DstOver => per(&|s, d, _, da| d + s * (1.0 - da)),
            Blend::SrcIn => per(&|s, _, _, da| s * da),
            Blend::DstIn => per(&|_, d, sa, _| d * sa),
            Blend::SrcOut => per(&|s, _, _, da| s * (1.0 - da)),
            Blend::DstOut => per(&|_, d, sa, _| d * (1.0 - sa)),
            Blend::SrcATop => F4 {
                r: s.r * d.a + d.r * (1.0 - s.a),
                g: s.g * d.a + d.g * (1.0 - s.a),
                b: s.b * d.a + d.b * (1.0 - s.a),
                a: d.a,
            },
            Blend::DstATop => F4 {
                r: d.r * s.a + s.r * (1.0 - d.a),
                g: d.g * s.a + s.g * (1.0 - d.a),
                b: d.b * s.a + s.b * (1.0 - d.a),
                a: s.a,
            },
            Blend::Xor => per(&|s, d, sa, da| s * (1.0 - da) + d * (1.0 - sa)),
            Blend::Plus => per(&|s, d, _, _| (s + d).min(1.0)),
            Blend::Modulate => per(&|s, d, _, _| s * d),
            Blend::Screen => per(&|s, d, _, _| s + d - s * d),
            Blend::Multiply => per(&|s, d, sa, da| s * (1.0 - da) + d * (1.0 - sa) + s * d),
            Blend::Darken => sep(&|s, d, sa, da| s + d - (s * da).max(d * sa)),
            Blend::Lighten => sep(&|s, d, sa, da| s + d - (s * da).min(d * sa)),
            Blend::Difference => sep(&|s, d, sa, da| s + d - 2.0 * (s * da).min(d * sa)),
            Blend::Exclusion => sep(&|s, d, _, _| s + d - 2.0 * s * d),
            Blend::Overlay => sep(&|s, d, sa, da| {
                s * (1.0 - da)
                    + d * (1.0 - sa)
                    + if 2.0 * d <= da {
                        2.0 * s * d
                    } else {
                        sa * da - 2.0 * (da - d) * (sa - s)
                    }
            }),
            Blend::HardLight => sep(&|s, d, sa, da| {
                s * (1.0 - da)
                    + d * (1.0 - sa)
                    + if 2.0 * s <= sa {
                        2.0 * s * d
                    } else {
                        sa * da - 2.0 * (da - d) * (sa - s)
                    }
            }),
            Blend::ColorDodge => sep(&|s, d, sa, da| {
                if d == 0.0 {
                    s * (1.0 - da)
                } else if s == sa {
                    s + d * (1.0 - sa)
                } else {
                    sa * da.min((d * sa) / (sa - s)) + s * (1.0 - da) + d * (1.0 - sa)
                }
            }),
            Blend::ColorBurn => sep(&|s, d, sa, da| {
                if d == da {
                    d + s * (1.0 - da)
                } else if s == 0.0 {
                    d * (1.0 - sa)
                } else {
                    sa * (da - da.min((da - d) * sa / s)) + s * (1.0 - da) + d * (1.0 - sa)
                }
            }),
            Blend::SoftLight => sep(&|s, d, sa, da| {
                let m = if da > 0.0 { d / da } else { 0.0 };
                let s2 = 2.0 * s;
                let m4 = 4.0 * m;
                let dark_src = d * (sa + (s2 - sa) * (1.0 - m));
                let dark_dst = (m4 * m4 + m4) * (m - 1.0) + 7.0 * m;
                let lite_dst = m.sqrt() - m;
                let lite_src =
                    d * sa + da * (s2 - sa) * if 4.0 * d <= da { dark_dst } else { lite_dst };
                s * (1.0 - da) + d * (1.0 - sa) + if s2 <= sa { dark_src } else { lite_src }
            }),
            Blend::Hue | Blend::Saturation | Blend::Color | Blend::Luminosity => {
                non_separable(self, s, d)
            }
        }
    }
}

fn lum(r: f32, g: f32, b: f32) -> f32 {
    r * 0.30 + g * 0.59 + b * 0.11
}

fn sat(r: f32, g: f32, b: f32) -> f32 {
    r.max(g).max(b) - r.min(g).min(b)
}

fn set_sat(r: &mut f32, g: &mut f32, b: &mut f32, s: f32) {
    let mn = r.min(*g).min(*b);
    let mx = r.max(*g).max(*b);
    let sat = mx - mn;
    let scale = |c: f32| if sat == 0.0 { 0.0 } else { (c - mn) * s / sat };
    *r = scale(*r);
    *g = scale(*g);
    *b = scale(*b);
}

fn set_lum(r: &mut f32, g: &mut f32, b: &mut f32, l: f32) {
    let diff = l - lum(*r, *g, *b);
    *r += diff;
    *g += diff;
    *b += diff;
}

fn clip_color(r: &mut f32, g: &mut f32, b: &mut f32, a: f32) {
    let mn = r.min(*g).min(*b);
    let mx = r.max(*g).max(*b);
    let l = lum(*r, *g, *b);
    let clip = |c: f32| {
        let mut c = c;
        if mn < 0.0 {
            c = l + (c - l) * l / (l - mn);
        }
        if mx > a {
            c = l + (c - l) * (a - l) / (mx - l);
        }
        c.max(0.0)
    };
    *r = clip(*r);
    *g = clip(*g);
    *b = clip(*b);
}

fn non_separable(mode: Blend, s: F4, d: F4) -> F4 {
    let (mut r, mut g, mut b);
    match mode {
        Blend::Hue => {
            r = s.r * s.a;
            g = s.g * s.a;
            b = s.b * s.a;
            set_sat(&mut r, &mut g, &mut b, sat(d.r, d.g, d.b) * s.a);
            set_lum(&mut r, &mut g, &mut b, lum(d.r, d.g, d.b) * s.a);
        }
        Blend::Saturation => {
            r = d.r * s.a;
            g = d.g * s.a;
            b = d.b * s.a;
            set_sat(&mut r, &mut g, &mut b, sat(s.r, s.g, s.b) * d.a);
            set_lum(&mut r, &mut g, &mut b, lum(d.r, d.g, d.b) * s.a);
        }
        Blend::Color => {
            r = s.r * d.a;
            g = s.g * d.a;
            b = s.b * d.a;
            set_lum(&mut r, &mut g, &mut b, lum(d.r, d.g, d.b) * s.a);
        }
        _ => {
            r = d.r * s.a;
            g = d.g * s.a;
            b = d.b * s.a;
            set_lum(&mut r, &mut g, &mut b, lum(s.r, s.g, s.b) * d.a);
        }
    }
    let a = s.a * d.a;
    clip_color(&mut r, &mut g, &mut b, a);
    F4 {
        r: d.r * (1.0 - s.a) + s.r * (1.0 - d.a) + r,
        g: d.g * (1.0 - s.a) + s.g * (1.0 - d.a) + g,
        b: d.b * (1.0 - s.a) + s.b * (1.0 - d.a) + b,
        a: s.a + d.a - a,
    }
}
