use criterion::{criterion_group, criterion_main, Criterion};
use ergotop::app::{App, View};
use ergotop::canvas::Canvas;
use ergotop::viz::{Visualizer, VizItem};
use ergotop_core::config::{AddressesFile, SourceSpec, UiConfig};
use ergotop_core::metrics::FEE_ADDRESS;
use ergotop_core::model::{BoxData, Input, SourceId, SourceKind, Tx};
use ergotop_core::sources::SourceEvent;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::Terminal;

const W: u16 = 200;
const H_CELLS: u16 = 50;

fn items(n: usize) -> Vec<VizItem> {
    let mut seed: u64 = 42;
    (0..n)
        .map(|i| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            VizItem {
                id: format!("{i:064}"),
                size_bytes: 200 + (seed >> 33) as u32 % 20_000,
                fee: 1_000_000 + (seed >> 20) % 10_000_000,
                color: Color::Rgb((seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8),
                pending: false,
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let txs = items(10_000);
    c.bench_function("relayout_10k", |b| {
        b.iter(|| {
            let mut v = Visualizer::new();
            v.set_size(W, H_CELLS * 2);
            v.relayout(&txs, 1_271_009, 0, true);
        })
    });
    let mut v = Visualizer::new();
    v.set_size(W, H_CELLS * 2);
    v.relayout(&txs, 1_271_009, 0, true);
    let area = Rect::new(0, 0, W, H_CELLS);
    c.bench_function("render_frame_10k", |b| {
        b.iter(|| {
            let mut canvas = Canvas::new(W, H_CELLS * 2);
            v.render(&mut canvas, 0, Color::Gray, Color::Blue);
            let mut buf = Buffer::empty(area);
            canvas.render(area, &mut buf);
        })
    });
    let mut falling = Visualizer::new();
    falling.set_size(W, H_CELLS * 2);
    falling.relayout(&txs, 1_271_009, 0, false);
    c.bench_function("render_frame_10k_falling", |b| {
        b.iter(|| {
            let mut canvas = Canvas::new(W, H_CELLS * 2);
            falling.render(&mut canvas, 300, Color::Gray, Color::Blue);
            let mut buf = Buffer::empty(area);
            canvas.render(area, &mut buf);
        })
    });
}

fn tx(i: usize, seed: u64) -> Tx {
    let bx = |address: &str, value: u64| BoxData {
        box_id: format!("{address}-{i}-{value}"),
        value,
        address: address.to_string(),
        tokens: vec![],
    };
    let input = bx(
        "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq",
        10_000_000_000,
    );
    Tx {
        id: format!("{i:064}"),
        size: 200 + (seed % 20_000) as u32,
        inputs: vec![Input {
            box_id: input.box_id.clone(),
            resolved: Some(input),
        }],
        outputs: vec![
            bx("4MQyMKvMbnCJG3aJ", 9_000_000_000),
            bx(FEE_ADDRESS, 1_000_000 + seed % 10_000_000),
        ],
        creation_ts_ms: None,
    }
}

/// A full Dashboard frame (`ui::draw`) with 10,000 txs in the pool, as spec §7 measures.
fn bench_dashboard(c: &mut Criterion) {
    let specs = vec![SourceSpec {
        id: SourceId("node".into()),
        kind: SourceKind::Node,
        url: "http://n".into(),
    }];
    let mut app = App::new(&specs, AddressesFile::default(), &UiConfig::default());
    app.view = View::Dashboard;
    let mut seed: u64 = 7;
    let txs: Vec<Tx> = (0..10_000)
        .map(|i| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            tx(i, seed >> 20)
        })
        .collect();
    let ids = txs.iter().map(|t| t.id.clone()).collect();
    app.on_source_event(
        SourceEvent::Mempool {
            source: SourceId("node".into()),
            ids,
            new_txs: txs,
            latency_ms: 1,
        },
        1_000,
    );
    c.bench_function("rows_10k", |b| b.iter(|| app.rows().len()));
    c.bench_function("rate_stats_10k", |b| b.iter(|| app.rate_stats()));
    let mut term = Terminal::new(TestBackend::new(200, 60)).unwrap();
    term.draw(|f| ergotop::ui::draw(f, &mut app, 2_000))
        .unwrap();
    c.bench_function("dashboard_frame_10k", |b| {
        b.iter(|| {
            term.draw(|f| ergotop::ui::draw(f, &mut app, 2_000))
                .unwrap();
        })
    });
    // Restart every sprite as a fresh fall (spawned at clock 1_000) and draw mid-fall.
    app.viz.relayout(&[], 1_271_009, 1_000, true);
    app.relayout(true);
    c.bench_function("dashboard_frame_10k_falling", |b| {
        b.iter(|| {
            term.draw(|f| ergotop::ui::draw(f, &mut app, 1_300))
                .unwrap();
        })
    });
}

criterion_group!(benches, bench, bench_dashboard);
criterion_main!(benches);
