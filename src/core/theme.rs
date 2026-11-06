//! Small, stable appearance presets. No filesystem access or per-frame work.
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ThemeId(u8);
impl ThemeId {
    pub fn new(id: u8) -> Self {
        Self(if id <= 30 { id } else { 0 })
    }
    pub fn index(self) -> u8 {
        self.0
    }
    pub fn resolve(self, system_dark: bool) -> Resolved {
        let id = if self.0 == 0 {
            if system_dark { 18 } else { 1 }
        } else {
            self.0
        };
        THEMES[(id - 1) as usize].resolve()
    }
}
impl<'de> Deserialize<'de> for ThemeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        Ok(Self::new(
            value.as_u64().filter(|n| *n <= 30).unwrap_or(0) as u8
        ))
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u32);
impl Color {
    pub fn rgb(self) -> [f64; 3] {
        [
            ((self.0 >> 16) & 255) as f64 / 255.,
            ((self.0 >> 8) & 255) as f64 / 255.,
            (self.0 & 255) as f64 / 255.,
        ]
    }
    pub fn mix(self, other: Self, t: f64) -> Self {
        let a = self.rgb();
        let b = other.rgb();
        let c = std::array::from_fn::<_, 3, _>(|i| {
            ((a[i] * (1. - t) + b[i] * t) * 255.).round() as u32
        });
        Self((c[0] << 16) | (c[1] << 8) | c[2])
    }
    pub fn contrast(self, other: Self) -> f64 {
        fn lum(c: Color) -> f64 {
            let v = c.rgb().map(|x| {
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            });
            v[0] * 0.2126 + v[1] * 0.7152 + v[2] * 0.0722
        }
        let a = lum(self);
        let b = lum(other);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconStyle {
    Classic,
    Duo,
    Pocket,
    Outline,
    Flat,
    Mono,
    Badge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Edge,
    Flat,
    Air,
    Lined,
    Soft,
    Compact,
    Cards,
    Rail,
    Header,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    pub dark: bool,
    pub icon: IconStyle,
    pub layout: Layout,
    pub colors: [Color; 8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub spec: &'static Theme,
    pub shell: Color,
    pub canvas: Color,
    pub rail: Color,
    pub ink: Color,
    pub muted: Color,
    pub line: Color,
    pub accent: Color,
    pub folder: Color,
    pub selection: Color,
    pub side_selection: Color,
    pub side_ink: Color,
    pub side_muted: Color,
    pub on_accent: Color,
}
impl Default for Resolved {
    fn default() -> Self {
        THEMES[0].resolve()
    }
}
impl Theme {
    pub fn resolve(&'static self) -> Resolved {
        let [
            shell,
            canvas,
            mut rail,
            ink,
            mut muted,
            line,
            accent,
            folder,
        ] = self.colors;
        let selection = canvas.mix(accent, if self.dark { 0.24 } else { 0.105 });
        for _ in 0..20 {
            if [canvas, rail, shell, selection]
                .iter()
                .all(|bg| muted.contrast(*bg) >= 4.6)
            {
                break;
            }
            muted = muted.mix(ink, 0.15);
        }
        let (side_ink, side_muted, side_selection) = if self.layout == Layout::Rail {
            rail = ink;
            (
                Color(0xedf1eb),
                Color(0xedf1eb),
                ink.mix(Color(0xffffff), 0.14),
            )
        } else {
            (
                ink,
                muted,
                rail.mix(accent, if self.dark { 0.23 } else { 0.12 }),
            )
        };
        let on_accent = if accent.contrast(Color(0xffffff)) >= 4.5 {
            Color(0xffffff)
        } else {
            Color(0x000000)
        };
        Resolved {
            spec: self,
            shell,
            canvas,
            rail,
            ink,
            muted,
            line,
            accent,
            folder,
            selection,
            side_selection,
            side_ink,
            side_muted,
            on_accent,
        }
    }
}
impl Resolved {
    pub fn radius(self) -> f64 {
        match self.spec.layout {
            Layout::Flat | Layout::Compact | Layout::Lined | Layout::Edge => 2.,
            Layout::Soft => 10.,
            _ => 6.,
        }
    }
    pub fn row_height(self, compact: bool) -> f64 {
        if compact || self.spec.layout == Layout::Compact {
            24.
        } else if self.spec.layout == Layout::Cards {
            34.
        } else {
            30.
        }
    }
}
pub static THEMES: [Theme; 30] = [
    Theme {
        id: 1,
        name: "익숙한 앰버",
        description: "Windows에 가까운 노란 폴더와 파란 선택색",
        dark: false,
        icon: IconStyle::Classic,
        layout: Layout::Edge,
        colors: [
            Color(0xf2f3f5),
            Color(0xffffff),
            Color(0xf6f7f9),
            Color(0x212a35),
            Color(0x667181),
            Color(0xdce1e7),
            Color(0x1766c1),
            Color(0xe8ac34),
        ],
    },
    Theme {
        id: 2,
        name: "소프트 실버",
        description: "중성 회색 화면에 따뜻한 폴더만 강조",
        dark: false,
        icon: IconStyle::Duo,
        layout: Layout::Flat,
        colors: [
            Color(0xededee),
            Color(0xfafafa),
            Color(0xf1f1f2),
            Color(0x292d33),
            Color(0x626a73),
            Color(0xd9dce0),
            Color(0x496079),
            Color(0xdcb86b),
        ],
    },
    Theme {
        id: 3,
        name: "네이티브 블루",
        description: "푸른 폴더와 여백으로 Mac의 익숙함 유지",
        dark: false,
        icon: IconStyle::Pocket,
        layout: Layout::Air,
        colors: [
            Color(0xf1f5f9),
            Color(0xffffff),
            Color(0xedf3f8),
            Color(0x243549),
            Color(0x5b7189),
            Color(0xd7e1eb),
            Color(0x1974c9),
            Color(0x4ca7df),
        ],
    },
    Theme {
        id: 4,
        name: "페이퍼 화이트",
        description: "밝은 문서 공간, 절제한 윤곽과 강조색",
        dark: false,
        icon: IconStyle::Outline,
        layout: Layout::Lined,
        colors: [
            Color(0xf8f8f6),
            Color(0xffffff),
            Color(0xf4f4f0),
            Color(0x30372f),
            Color(0x697064),
            Color(0xdde0d6),
            Color(0x527c49),
            Color(0xd2a345),
        ],
    },
    Theme {
        id: 5,
        name: "클래식 10",
        description: "각진 모서리와 촘촘한 행으로 익숙하게",
        dark: false,
        icon: IconStyle::Classic,
        layout: Layout::Compact,
        colors: [
            Color(0xf0f0f0),
            Color(0xffffff),
            Color(0xf5f5f5),
            Color(0x212121),
            Color(0x626262),
            Color(0xd9d9d9),
            Color(0x075ca8),
            Color(0xe6b331),
        ],
    },
    Theme {
        id: 6,
        name: "세이지 데스크",
        description: "눈에 편한 연녹색과 부드러운 골드",
        dark: false,
        icon: IconStyle::Duo,
        layout: Layout::Soft,
        colors: [
            Color(0xeef1e9),
            Color(0xfafcf7),
            Color(0xe9eee3),
            Color(0x2b382c),
            Color(0x697760),
            Color(0xd5dfcc),
            Color(0x547e45),
            Color(0xccad5f),
        ],
    },
    Theme {
        id: 7,
        name: "샌드 스톤",
        description: "크림 배경과 선명한 캐러멜 폴더",
        dark: false,
        icon: IconStyle::Pocket,
        layout: Layout::Air,
        colors: [
            Color(0xf0ebe3),
            Color(0xfffdf8),
            Color(0xf1ece1),
            Color(0x41392e),
            Color(0x7c7162),
            Color(0xe1d6c6),
            Color(0x976138),
            Color(0xd59a45),
        ],
    },
    Theme {
        id: 8,
        name: "포그 라벤더",
        description: "회색에 가까운 보라, 또렷한 검정 글자",
        dark: false,
        icon: IconStyle::Duo,
        layout: Layout::Soft,
        colors: [
            Color(0xefedf4),
            Color(0xfdfcff),
            Color(0xefecf5),
            Color(0x373146),
            Color(0x776d87),
            Color(0xded7e8),
            Color(0x75619a),
            Color(0xb29ad4),
        ],
    },
    Theme {
        id: 9,
        name: "시 글라스",
        description: "맑은 청록 화면과 구별되는 황금 폴더",
        dark: false,
        icon: IconStyle::Flat,
        layout: Layout::Air,
        colors: [
            Color(0xeaf2f1),
            Color(0xfbfffe),
            Color(0xe6f0ee),
            Color(0x233f3b),
            Color(0x5c7771),
            Color(0xcddfda),
            Color(0x257e71),
            Color(0xd3a33f),
        ],
    },
    Theme {
        id: 10,
        name: "로즈 페이퍼",
        description: "분홍을 아주 얕게 쓰고 화면은 담백하게",
        dark: false,
        icon: IconStyle::Pocket,
        layout: Layout::Soft,
        colors: [
            Color(0xf4edef),
            Color(0xfffafb),
            Color(0xf5e9ed),
            Color(0x4a3139),
            Color(0x816875),
            Color(0xe7d4dc),
            Color(0xa35070),
            Color(0xd9a16a),
        ],
    },
    Theme {
        id: 11,
        name: "인디고 포커스",
        description: "색은 선택에만, 글자와 파일은 또렷하게",
        dark: false,
        icon: IconStyle::Outline,
        layout: Layout::Edge,
        colors: [
            Color(0xedf0f8),
            Color(0xffffff),
            Color(0xf0f2f9),
            Color(0x202c48),
            Color(0x586987),
            Color(0xd5dcec),
            Color(0x3758b3),
            Color(0xd2a12e),
        ],
    },
    Theme {
        id: 12,
        name: "잉크 & 골드",
        description: "짙은 탐색 영역과 밝은 파일 목록의 대비",
        dark: false,
        icon: IconStyle::Classic,
        layout: Layout::Rail,
        colors: [
            Color(0xf1f1ed),
            Color(0xfffef9),
            Color(0xf0f0e8),
            Color(0x26332e),
            Color(0x627367),
            Color(0xd9ded1),
            Color(0x507249),
            Color(0xd9ac40),
        ],
    },
    Theme {
        id: 13,
        name: "블루 프린트",
        description: "뚜렷한 파란 윤곽으로 파일 형태 강조",
        dark: false,
        icon: IconStyle::Outline,
        layout: Layout::Flat,
        colors: [
            Color(0xeaf2fa),
            Color(0xffffff),
            Color(0xe8f1fa),
            Color(0x193c60),
            Color(0x577694),
            Color(0xc9dceb),
            Color(0x116eb8),
            Color(0x3d95c7),
        ],
    },
    Theme {
        id: 14,
        name: "티얼 시그널",
        description: "청록 선택선과 금색 폴더로 빠르게 구분",
        dark: false,
        icon: IconStyle::Flat,
        layout: Layout::Edge,
        colors: [
            Color(0xedf2f2),
            Color(0xffffff),
            Color(0xe8f0ef),
            Color(0x203c3c),
            Color(0x5d7775),
            Color(0xd0dfdd),
            Color(0x14786e),
            Color(0xd9a328),
        ],
    },
    Theme {
        id: 15,
        name: "모노 앰버",
        description: "거의 무채색인 화면에 폴더만 색을 사용",
        dark: false,
        icon: IconStyle::Mono,
        layout: Layout::Lined,
        colors: [
            Color(0xefefee),
            Color(0xffffff),
            Color(0xf2f2f0),
            Color(0x242625),
            Color(0x626662),
            Color(0xd9dcd7),
            Color(0x756238),
            Color(0xd1a23b),
        ],
    },
    Theme {
        id: 16,
        name: "슬레이트 나이트",
        description: "푸른 회색의 어두운 화면, 폴더는 앰버",
        dark: true,
        icon: IconStyle::Duo,
        layout: Layout::Edge,
        colors: [
            Color(0x222b38),
            Color(0x19222e),
            Color(0x202c3a),
            Color(0xe8eef7),
            Color(0xa4b4c8),
            Color(0x354354),
            Color(0x72b8ef),
            Color(0xedbd5c),
        ],
    },
    Theme {
        id: 17,
        name: "그래파이트",
        description: "중성 회색과 풍부한 아이콘 색상",
        dark: true,
        icon: IconStyle::Classic,
        layout: Layout::Flat,
        colors: [
            Color(0x282a2d),
            Color(0x1f2124),
            Color(0x25282b),
            Color(0xeeeeef),
            Color(0xb0b6bd),
            Color(0x41454b),
            Color(0xa5bbc9),
            Color(0xe4b75c),
        ],
    },
    Theme {
        id: 18,
        name: "미드나이트",
        description: "남색 바탕에서 금색 폴더와 파일 윤곽 강조",
        dark: true,
        icon: IconStyle::Pocket,
        layout: Layout::Soft,
        colors: [
            Color(0x1c2839),
            Color(0x142031),
            Color(0x1a293e),
            Color(0xe7effb),
            Color(0xa3b6cf),
            Color(0x30435d),
            Color(0x80b9f1),
            Color(0xedc36d),
        ],
    },
    Theme {
        id: 19,
        name: "다크 포레스트",
        description: "녹색 기운을 낮게 깐 편안한 어두운 화면",
        dark: true,
        icon: IconStyle::Duo,
        layout: Layout::Air,
        colors: [
            Color(0x26312d),
            Color(0x1c2723),
            Color(0x23332a),
            Color(0xe5eee6),
            Color(0xa7bcae),
            Color(0x3c5142),
            Color(0x90c6a1),
            Color(0xd4bd77),
        ],
    },
    Theme {
        id: 20,
        name: "에스프레소",
        description: "갈색과 구리색으로 차가운 느낌을 줄임",
        dark: true,
        icon: IconStyle::Classic,
        layout: Layout::Lined,
        colors: [
            Color(0x302823),
            Color(0x231e1b),
            Color(0x2d2620),
            Color(0xf0e8df),
            Color(0xbdad9f),
            Color(0x4e4035),
            Color(0xddaf85),
            Color(0xe2b166),
        ],
    },
    Theme {
        id: 21,
        name: "컬러 태그",
        description: "파일 종류 색상과 큰 확장자 표식",
        dark: false,
        icon: IconStyle::Badge,
        layout: Layout::Soft,
        colors: [
            Color(0xf2f4f6),
            Color(0xffffff),
            Color(0xeef2f5),
            Color(0x283744),
            Color(0x647584),
            Color(0xd9e1e7),
            Color(0x357ca2),
            Color(0xe2ae41),
        ],
    },
    Theme {
        id: 22,
        name: "아웃라인 프로",
        description: "가벼운 면적, 굵고 분명한 아이콘 윤곽",
        dark: false,
        icon: IconStyle::Outline,
        layout: Layout::Flat,
        colors: [
            Color(0xf2f4f3),
            Color(0xffffff),
            Color(0xedf2ef),
            Color(0x263c32),
            Color(0x647a6c),
            Color(0xd4e0d7),
            Color(0x387456),
            Color(0x5c936b),
        ],
    },
    Theme {
        id: 23,
        name: "포켓 폴더",
        description: "폴더의 열린 입구와 파일의 접힌 모서리",
        dark: false,
        icon: IconStyle::Pocket,
        layout: Layout::Cards,
        colors: [
            Color(0xf5f0e8),
            Color(0xfffcf7),
            Color(0xf5eee2),
            Color(0x40392d),
            Color(0x7b705f),
            Color(0xe5d9c7),
            Color(0x916c3c),
            Color(0xd4a451),
        ],
    },
    Theme {
        id: 24,
        name: "플랫 마커",
        description: "진한 파일 종류 타일, 단순한 폴더 형태",
        dark: false,
        icon: IconStyle::Badge,
        layout: Layout::Edge,
        colors: [
            Color(0xeef1f6),
            Color(0xffffff),
            Color(0xeaf0f7),
            Color(0x26354e),
            Color(0x5f718d),
            Color(0xd4deed),
            Color(0x3d6fae),
            Color(0x619ed8),
        ],
    },
    Theme {
        id: 25,
        name: "나이트 마커",
        description: "어두운 배경에서도 타입 표식이 명확하게",
        dark: true,
        icon: IconStyle::Badge,
        layout: Layout::Cards,
        colors: [
            Color(0x272a33),
            Color(0x1c2029),
            Color(0x252d3a),
            Color(0xe9eef7),
            Color(0xabb7ca),
            Color(0x3c485a),
            Color(0x98badf),
            Color(0xedbd64),
        ],
    },
    Theme {
        id: 26,
        name: "라이트 리본",
        description: "가벼운 색의 상단과 정돈된 목록",
        dark: false,
        icon: IconStyle::Flat,
        layout: Layout::Header,
        colors: [
            Color(0xedf2f9),
            Color(0xffffff),
            Color(0xeef3f8),
            Color(0x293c54),
            Color(0x61758e),
            Color(0xd5e1ed),
            Color(0x2a699d),
            Color(0xe2b85b),
        ],
    },
    Theme {
        id: 27,
        name: "컴팩트 그리드",
        description: "작업량이 많을 때 편한 밀도와 구분선",
        dark: false,
        icon: IconStyle::Duo,
        layout: Layout::Compact,
        colors: [
            Color(0xeef0ef),
            Color(0xffffff),
            Color(0xeff2ef),
            Color(0x27382d),
            Color(0x647669),
            Color(0xd5ddd5),
            Color(0x4c7758),
            Color(0xb7a05d),
        ],
    },
    Theme {
        id: 28,
        name: "웜 카드",
        description: "행마다 독립된 영역으로 대상을 분명하게",
        dark: false,
        icon: IconStyle::Classic,
        layout: Layout::Cards,
        colors: [
            Color(0xf1ede6),
            Color(0xfaf8f3),
            Color(0xeee8dc),
            Color(0x41392f),
            Color(0x7c7161),
            Color(0xded5c5),
            Color(0x92733c),
            Color(0xd9ab57),
        ],
    },
    Theme {
        id: 29,
        name: "블루 아워",
        description: "살짝 분리된 사이드바와 푸른 폴더",
        dark: true,
        icon: IconStyle::Pocket,
        layout: Layout::Air,
        colors: [
            Color(0x292e3f),
            Color(0x202536),
            Color(0x2b344a),
            Color(0xe8ecf8),
            Color(0xadb8d2),
            Color(0x424d68),
            Color(0xa5b8ed),
            Color(0x83b0e1),
        ],
    },
    Theme {
        id: 30,
        name: "차콜 & 라임",
        description: "무채색 작업 영역에 연두색 선택선",
        dark: true,
        icon: IconStyle::Outline,
        layout: Layout::Edge,
        colors: [
            Color(0x2a2d29),
            Color(0x20231f),
            Color(0x272d24),
            Color(0xe9eee2),
            Color(0xb1bda5),
            Color(0x424c3a),
            Color(0xb1ce82),
            Color(0xc3cc7b),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preset_is_legible_including_selection_and_sidebar() {
        for (i, t) in THEMES.iter().enumerate() {
            assert_eq!(t.id as usize, i + 1);
            let p = t.resolve();
            for (fg, bg) in [
                (p.ink, p.canvas),
                (p.ink, p.selection),
                (p.muted, p.canvas),
                (p.muted, p.selection),
                (p.muted, p.shell),
                (p.side_muted, p.rail),
                (p.side_ink, p.side_selection),
                (p.on_accent, p.accent),
            ] {
                assert!(
                    fg.contrast(bg) >= 4.5,
                    "{}: {:?}/{:?}: {}",
                    t.id,
                    fg,
                    bg,
                    fg.contrast(bg)
                );
            }
        }
    }
    #[test]
    fn unknown_choices_fall_back_without_erasing_other_preferences() {
        for id in [
            serde_json::json!(99),
            serde_json::json!(-1),
            serde_json::json!("removed"),
            serde_json::json!(null),
        ] {
            let p: crate::core::model::Preferences = serde_json::from_value(
                serde_json::json!({"theme":id,"keyboard":"Finder","hidden":true}),
            )
            .unwrap();
            assert_eq!(p.theme, ThemeId::default());
            assert!(p.hidden);
            assert_eq!(p.keyboard, crate::core::model::KeyboardProfile::Finder);
        }
    }
    #[test]
    fn system_follows_appearance_but_explicit_choice_does_not() {
        assert_eq!(ThemeId::default().resolve(false).spec.id, 1);
        assert_eq!(ThemeId::default().resolve(true).spec.id, 18);
        for id in 1..=30 {
            assert_eq!(
                ThemeId::new(id).resolve(false),
                ThemeId::new(id).resolve(true)
            );
        }
    }
}
