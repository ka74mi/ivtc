use ivtc_core::features::FieldOrder;
use ivtc_core::nv12::Nv12Layout;

pub const DEFAULT_CORING: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub layout: Nv12Layout,
    pub order: FieldOrder,
    pub coring: u8,
}

fn parse_number<T: std::str::FromStr>(name: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{name} の値が不正です: {value}"))
}

pub fn parse(args: &[String]) -> Result<Config, String> {
    let mut width = None;
    let mut height = None;
    let mut order = None;
    let mut coring = DEFAULT_CORING;

    let mut iter = args.iter();
    while let Some(name) = iter.next() {
        let value = iter
            .next()
            .ok_or_else(|| format!("{name} に値がありません"))?;
        match name.as_str() {
            "--width" => width = Some(parse_number::<u32>(name, value)?),
            "--height" => height = Some(parse_number::<u32>(name, value)?),
            "--field-order" => {
                order = Some(match value.as_str() {
                    "tff" => FieldOrder::TopFirst,
                    "bff" => FieldOrder::BottomFirst,
                    _ => return Err(format!("{name} は tff か bff を指定してください: {value}")),
                });
            }
            "--coring" => coring = parse_number::<u8>(name, value)?,
            _ => return Err(format!("不明なオプションです: {name}")),
        }
    }

    let width = width.ok_or("--width が必要です")?;
    let height = height.ok_or("--height が必要です")?;
    let order = order.ok_or("--field-order が必要です")?;
    let layout = Nv12Layout::packed(width, height)
        .map_err(|e| format!("画像サイズが不正です ({width}x{height}): {e:?}"))?;

    Ok(Config {
        layout,
        order,
        coring,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parses_required_options_with_default_coring() {
        let c = parse(&args(&[
            "--width",
            "1440",
            "--height",
            "1080",
            "--field-order",
            "tff",
        ]))
        .unwrap();
        assert_eq!(c.layout, Nv12Layout::packed(1440, 1080).unwrap());
        assert_eq!(c.order, FieldOrder::TopFirst);
        assert_eq!(c.coring, DEFAULT_CORING);
    }

    #[test]
    fn parses_bff_and_explicit_coring_in_any_order() {
        let c = parse(&args(&[
            "--coring",
            "9",
            "--field-order",
            "bff",
            "--height",
            "1080",
            "--width",
            "1920",
        ]))
        .unwrap();
        assert_eq!(c.order, FieldOrder::BottomFirst);
        assert_eq!(c.coring, 9);
        assert_eq!(c.layout.width, 1920);
    }

    #[test]
    fn rejects_missing_unknown_and_invalid_options() {
        assert!(parse(&args(&["--width", "8", "--height", "8"])).is_err());
        assert!(parse(&args(&["--width"])).is_err());
        assert!(parse(&args(&["--bogus", "1"])).is_err());
        assert!(
            parse(&args(&[
                "--width",
                "8",
                "--height",
                "8",
                "--field-order",
                "xyz"
            ]))
            .is_err()
        );
        assert!(
            parse(&args(&[
                "--width",
                "7",
                "--height",
                "8",
                "--field-order",
                "tff"
            ]))
            .is_err()
        );
        assert!(
            parse(&args(&[
                "--width",
                "8",
                "--height",
                "8",
                "--field-order",
                "tff",
                "--coring",
                "300"
            ]))
            .is_err()
        );
    }
}
