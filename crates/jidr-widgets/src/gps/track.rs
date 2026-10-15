//! GPX and KML tracks: just enough parsing for route replay (track, route
//! and way points; KML `<coordinates>` and `gx:coord`), plus geometry for
//! walking a polyline at a given speed.

use jidr_core::{JidrError, JidrResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
}

/// Value of `name="…"` inside one tag's text.
fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=");
    let i = tag.find(&key)? + key.len();
    let rest = &tag[i..];
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let end = rest[1..].find(q)?;
    rest[1..1 + end].trim().parse().ok()
}

/// Text between `<name…>` and `</name>` right after `from`.
fn child_text<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = s.find(&format!("<{name}"))?;
    let gt = s[open..].find('>')? + open + 1;
    let close = s[gt..].find(&format!("</{name}>"))? + gt;
    Some(s[gt..close].trim())
}

fn valid(p: &Point) -> bool {
    (-90.0..=90.0).contains(&p.lat) && (-180.0..=180.0).contains(&p.lon)
}

fn parse_gpx(text: &str) -> Vec<Point> {
    let mut out = Vec::new();
    for tag in ["trkpt", "rtept", "wpt"] {
        let open = format!("<{tag}");
        let mut rest = text;
        while let Some(i) = rest.find(&open) {
            let after = &rest[i..];
            let Some(gt) = after.find('>') else { break };
            let head = &after[..gt];
            // The point's body runs to its closing tag (or it self-closes).
            let body_end = if head.ends_with('/') {
                gt
            } else {
                after.find(&format!("</{tag}>")).unwrap_or(gt)
            };
            let body = &after[gt..body_end];
            if let (Some(lat), Some(lon)) = (attr(head, "lat"), attr(head, "lon")) {
                let alt = child_text(body, "ele")
                    .and_then(|e| e.parse().ok())
                    .unwrap_or(0.0);
                out.push(Point { lat, lon, alt });
            }
            rest = &after[gt.max(1)..];
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

fn parse_kml(text: &str) -> Vec<Point> {
    let mut out = Vec::new();
    // <coordinates>lon,lat[,alt] lon,lat[,alt] …</coordinates>
    let mut rest = text;
    while let Some(c) = child_text(rest, "coordinates") {
        for tuple in c.split_whitespace() {
            let v: Vec<f64> = tuple
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect();
            if v.len() >= 2 {
                out.push(Point {
                    lat: v[1],
                    lon: v[0],
                    alt: v.get(2).copied().unwrap_or(0.0),
                });
            }
        }
        let Some(i) = rest.find("</coordinates>") else {
            break;
        };
        rest = &rest[i + 14..];
    }
    if out.is_empty() {
        // gx:Track: <gx:coord>lon lat alt</gx:coord>
        let mut rest = text;
        while let Some(c) = child_text(rest, "gx:coord") {
            let v: Vec<f64> = c
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() >= 2 {
                out.push(Point {
                    lat: v[1],
                    lon: v[0],
                    alt: v.get(2).copied().unwrap_or(0.0),
                });
            }
            let Some(i) = rest.find("</gx:coord>") else {
                break;
            };
            rest = &rest[i + 11..];
        }
    }
    out
}

/// Points of a GPX or KML file, in order.
pub fn parse(text: &str) -> JidrResult<Vec<Point>> {
    let pts: Vec<Point> = if text.contains("<kml") {
        parse_kml(text)
    } else {
        parse_gpx(text)
    };
    let pts: Vec<Point> = pts.into_iter().filter(valid).collect();
    if pts.is_empty() {
        return Err(JidrError::usage(
            "no track, route or way points in the file",
        ));
    }
    Ok(pts)
}

const EARTH_M: f64 = 6_371_000.0;

/// Great-circle distance in metres.
pub fn distance(a: Point, b: Point) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_M * h.sqrt().asin()
}

/// Initial bearing a → b, degrees clockwise from north.
pub fn bearing(a: Point, b: Point) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// A track with cumulative distances, for walking it.
#[derive(Debug, Clone)]
pub struct Track {
    pub points: Vec<Point>,
    cum: Vec<f64>,
}

impl Track {
    pub fn new(points: Vec<Point>) -> Self {
        let mut cum = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d += distance(points[i - 1], *p);
            }
            cum.push(d);
        }
        Self { points, cum }
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Position and heading `d` metres along the track (clamped).
    pub fn at(&self, d: f64) -> (Point, f64) {
        let n = self.points.len();
        if n == 1 || d <= 0.0 {
            let b = if n > 1 {
                bearing(self.points[0], self.points[1])
            } else {
                0.0
            };
            return (self.points[0], b);
        }
        let d = d.min(self.length());
        let i = self.cum.partition_point(|c| *c <= d).clamp(1, n - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let seg = (self.cum[i] - self.cum[i - 1]).max(1e-9);
        let t = ((d - self.cum[i - 1]) / seg).clamp(0.0, 1.0);
        let p = Point {
            lat: a.lat + (b.lat - a.lat) * t,
            lon: a.lon + (b.lon - a.lon) * t,
            alt: a.alt + (b.alt - a.alt) * t,
        };
        (p, bearing(a, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0"?><gpx version="1.1"><trk><trkseg>
      <trkpt lat="32.8954" lon="13.1802"><ele>21</ele></trkpt>
      <trkpt lat='32.8900' lon='13.1850'><ele>25</ele><time>2026-10-13T14:00:00Z</time></trkpt>
      <trkpt lat="32.6635" lon="13.1590"/>
    </trkseg></trk></gpx>"#;

    #[test]
    fn parses_gpx_track_points_in_order() {
        let p = parse(GPX).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].lat, p[0].lon, p[0].alt), (32.8954, 13.1802, 21.0));
        assert_eq!(p[1].alt, 25.0);
        assert_eq!(p[2].lat, 32.6635);
    }

    #[test]
    fn parses_kml_coordinates_and_gx_track() {
        let kml = "<kml><Placemark><LineString><coordinates>13.1802,32.8954,21 13.1850,32.8900</coordinates></LineString></Placemark></kml>";
        let p = parse(kml).unwrap();
        assert_eq!((p[0].lat, p[0].lon, p[0].alt), (32.8954, 13.1802, 21.0));
        assert_eq!(p[1].lon, 13.1850);
        let gx = "<kml><gx:Track><gx:coord>13.1 32.8 5</gx:coord><gx:coord>13.2 32.9 6</gx:coord></gx:Track></kml>";
        assert_eq!(parse(gx).unwrap().len(), 2);
        assert!(parse("<gpx></gpx>").is_err());
        assert!(parse("<gpx><wpt lat=\"999\" lon=\"0\"/></gpx>").is_err());
    }

    #[test]
    fn walks_the_track() {
        let t = Track::new(parse(GPX).unwrap());
        assert!(
            t.length() > 25_000.0 && t.length() < 27_000.0,
            "{}",
            t.length()
        );
        let (start, _) = t.at(0.0);
        assert_eq!(start.lat, 32.8954);
        let (end, _) = t.at(1e9);
        assert_eq!(end.lat, 32.6635);
        let (mid, b) = t.at(t.length() / 2.0);
        assert!(mid.lat < 32.89 && mid.lat > 32.66);
        assert!((150.0..=200.0).contains(&b), "south-ish: {b}");
        let a = Point {
            lat: 0.0,
            lon: 0.0,
            alt: 0.0,
        };
        assert!((bearing(a, Point { lat: 1.0, ..a }) - 0.0).abs() < 1e-6);
        assert!((bearing(a, Point { lon: 1.0, ..a }) - 90.0).abs() < 1e-6);
    }
}
