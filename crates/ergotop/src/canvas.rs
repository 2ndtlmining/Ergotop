//! Half-block pixel canvas: each terminal cell shows two vertical pixels.
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;

pub struct Canvas {
    width: u16,
    height: u16,
    px: Vec<Option<Color>>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Self {
        Canvas {
            width,
            height,
            px: vec![None; width as usize * height as usize],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }

    pub fn get(&self, x: u16, y: u16) -> Option<Color> {
        self.index(x as i32, y as i32).and_then(|i| self.px[i])
    }

    pub fn set(&mut self, x: i32, y: i32, c: Color) {
        if let Some(i) = self.index(x, y) {
            self.px[i] = Some(c);
        }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: u16, h: u16, c: Color) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.set(x + dx, y + dy, c);
            }
        }
    }

    fn at(&self, x: u16, y: i32) -> Option<Color> {
        self.index(x as i32, y).and_then(|i| self.px[i])
    }

    /// Draws into `area`; pixel rows map bottom-up, two per terminal row.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let rows = area.height.min(self.height.div_ceil(2));
        let cols = area.width.min(self.width);
        for cy in 0..rows {
            let top_y = self.height as i32 - 1 - 2 * cy as i32;
            for cx in 0..cols {
                let top = self.at(cx, top_y);
                let bottom = self.at(cx, top_y - 1);
                let Some(cell) = buf.cell_mut(Position::new(area.x + cx, area.y + cy)) else {
                    continue;
                };
                match (top, bottom) {
                    (None, None) => {}
                    (Some(t), None) => {
                        cell.set_char('▀').set_fg(t);
                    }
                    (None, Some(b)) => {
                        cell.set_char('▄').set_fg(b);
                    }
                    (Some(t), Some(b)) => {
                        cell.set_char('▀').set_fg(t).set_bg(b);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::Rgb(200, 0, 0);
    const BLUE: Color = Color::Rgb(0, 0, 200);

    fn rendered(c: &Canvas, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        c.render(area, &mut buf);
        buf
    }

    #[test]
    fn top_pixel_is_upper_half_block() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 1, RED);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!(b[(0, 0)].fg, RED);
    }

    #[test]
    fn bottom_pixel_is_lower_half_block() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 0, RED);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▄");
        assert_eq!(b[(0, 0)].fg, RED);
    }

    #[test]
    fn both_pixels_use_fg_and_bg() {
        let mut c = Canvas::new(1, 2);
        c.set(0, 1, RED);
        c.set(0, 0, BLUE);
        let b = rendered(&c, 1, 1);
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!((b[(0, 0)].fg, b[(0, 0)].bg), (RED, BLUE));
    }

    #[test]
    fn rows_map_bottom_up_and_empty_cells_are_untouched() {
        let mut c = Canvas::new(1, 4);
        c.set(0, 0, RED);
        let b = rendered(&c, 1, 2);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "▄");
    }

    #[test]
    fn writes_outside_the_canvas_are_clipped() {
        let mut c = Canvas::new(3, 3);
        c.set(-1, 0, RED);
        c.set(5, 5, RED);
        c.fill(2, 2, 4, 4, RED);
        assert_eq!(c.get(2, 2), Some(RED));
        assert_eq!(c.get(1, 1), None);
        assert_eq!(c.get(9, 9), None);
    }
}
