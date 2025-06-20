//! Performance benchmarks for terminal operations
//! 
//! These benchmarks measure the performance of critical terminal operations
//! to detect performance regressions and establish baseline metrics.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use terminal::{Terminal, TerminalBuilder};
use std::time::Duration;

fn benchmark_terminal_creation(c: &mut Criterion) {
    c.bench_function("terminal_creation", |b| {
        b.iter(|| {
            let terminal = black_box(TerminalBuilder::new().build());
            terminal
        })
    });
}

fn benchmark_terminal_input_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("terminal_input");
    
    // Test different input sizes
    for size in [10, 100, 1000, 10000].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::new("process_input", size), size, |b, &size| {
            let mut terminal = TerminalBuilder::new().build();
            let input = "a".repeat(size);
            
            b.iter(|| {
                terminal.input(black_box(input.as_bytes()));
            });
        });
    }
    group.finish();
}

fn benchmark_terminal_output_rendering(c: &mut Criterion) {
    let mut group = c.benchmark_group("terminal_output");
    
    // Test different output scenarios
    let test_cases = vec![
        ("simple_text", "Hello, World!\n".repeat(100)),
        ("ansi_colors", "\x1b[31mRed\x1b[32mGreen\x1b[34mBlue\x1b[0m\n".repeat(50)),
        ("complex_escape", "\x1b[2J\x1b[H\x1b[31;1mBold Red\x1b[0m\n".repeat(25)),
    ];
    
    for (name, output) in test_cases {
        group.bench_function(name, |b| {
            let mut terminal = TerminalBuilder::new().build();
            
            b.iter(|| {
                terminal.input(black_box(output.as_bytes()));
                // Simulate rendering
                let _ = terminal.renderable_content();
            });
        });
    }
    group.finish();
}

fn benchmark_terminal_scrolling(c: &mut Criterion) {
    let mut group = c.benchmark_group("terminal_scrolling");
    
    // Setup terminal with content
    let mut terminal = TerminalBuilder::new().build();
    for i in 0..1000 {
        terminal.input(format!("Line {}\n", i).as_bytes());
    }
    
    group.bench_function("scroll_up", |b| {
        b.iter(|| {
            terminal.scroll_display(black_box(-10));
        });
    });
    
    group.bench_function("scroll_down", |b| {
        b.iter(|| {
            terminal.scroll_display(black_box(10));
        });
    });
    
    group.bench_function("scroll_to_top", |b| {
        b.iter(|| {
            terminal.scroll_display(black_box(i32::MIN));
        });
    });
    
    group.bench_function("scroll_to_bottom", |b| {
        b.iter(|| {
            terminal.scroll_display(black_box(i32::MAX));
        });
    });
    
    group.finish();
}

fn benchmark_terminal_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("terminal_search");
    
    // Setup terminal with searchable content
    let mut terminal = TerminalBuilder::new().build();
    let content = "The quick brown fox jumps over the lazy dog.\n".repeat(100);
    terminal.input(content.as_bytes());
    
    let search_terms = vec![
        ("short_term", "fox"),
        ("medium_term", "quick brown"),
        ("long_term", "jumps over the lazy"),
        ("not_found", "nonexistent"),
    ];
    
    for (name, term) in search_terms {
        group.bench_function(name, |b| {
            b.iter(|| {
                // Simulate search operation
                let content = terminal.renderable_content();
                let _ = content.display_iter().any(|indexed| {
                    indexed.cell.c.to_string().contains(black_box(term))
                });
            });
        });
    }
    
    group.finish();
}

fn benchmark_terminal_resize(c: &mut Criterion) {
    let mut group = c.benchmark_group("terminal_resize");
    
    let resize_scenarios = vec![
        ("small_to_large", (40, 20), (120, 40)),
        ("large_to_small", (120, 40), (40, 20)),
        ("width_only", (80, 24), (120, 24)),
        ("height_only", (80, 24), (80, 40)),
    ];
    
    for (name, (from_size, to_size)) in resize_scenarios {
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut terminal = TerminalBuilder::new()
                    .columns(from_size.0)
                    .lines(from_size.1)
                    .build();
                
                // Add some content
                terminal.input(b"Hello, World!\nThis is a test.\n");
                
                // Perform resize
                terminal.resize(black_box(to_size.0), black_box(to_size.1));
            });
        });
    }
    
    group.finish();
}

fn benchmark_terminal_hyperlink_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("hyperlink_detection");
    
    let test_inputs = vec![
        ("no_links", "This is plain text with no links at all.\n".repeat(50)),
        ("single_link", "Check out https://example.com for more info.\n".repeat(50)),
        ("multiple_links", "Visit https://example.com and http://test.org and ftp://files.com\n".repeat(20)),
        ("mixed_content", "Some text https://example.com more text http://test.org end.\n".repeat(30)),
    ];
    
    for (name, input) in test_inputs {
        group.bench_function(name, |b| {
            let mut terminal = TerminalBuilder::new().build();
            
            b.iter(|| {
                terminal.input(black_box(input.as_bytes()));
                // Simulate hyperlink detection
                let _ = terminal.renderable_content();
            });
        });
    }
    
    group.finish();
}

fn benchmark_terminal_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_usage");
    
    group.bench_function("large_buffer", |b| {
        b.iter(|| {
            let mut terminal = TerminalBuilder::new()
                .scrollback_lines(10000)
                .build();
            
            // Fill terminal with content
            for i in 0..5000 {
                terminal.input(black_box(format!("Line {} with some content to fill memory\n", i).as_bytes()));
            }
            
            // Measure memory-intensive operations
            let _ = terminal.renderable_content();
        });
    });
    
    group.bench_function("rapid_updates", |b| {
        let mut terminal = TerminalBuilder::new().build();
        
        b.iter(|| {
            // Simulate rapid terminal updates
            for _ in 0..100 {
                terminal.input(black_box(b"Update\r"));
            }
        });
    });
    
    group.finish();
}

fn benchmark_terminal_concurrent_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_operations");
    
    group.bench_function("input_and_render", |b| {
        let mut terminal = TerminalBuilder::new().build();
        
        b.iter(|| {
            // Simulate concurrent input and rendering
            terminal.input(black_box(b"Input data\n"));
            let _ = terminal.renderable_content();
            terminal.input(black_box(b"More input\n"));
            let _ = terminal.renderable_content();
        });
    });
    
    group.bench_function("scroll_and_input", |b| {
        let mut terminal = TerminalBuilder::new().build();
        
        // Pre-fill with content
        for i in 0..100 {
            terminal.input(format!("Line {}\n", i).as_bytes());
        }
        
        b.iter(|| {
            terminal.scroll_display(black_box(-5));
            terminal.input(black_box(b"New input\n"));
            terminal.scroll_display(black_box(5));
        });
    });
    
    group.finish();
}

// Custom benchmark configuration
fn custom_criterion() -> Criterion {
    Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .sample_size(100)
        .warm_up_time(Duration::from_secs(2))
}

criterion_group! {
    name = terminal_benchmarks;
    config = custom_criterion();
    targets = 
        benchmark_terminal_creation,
        benchmark_terminal_input_processing,
        benchmark_terminal_output_rendering,
        benchmark_terminal_scrolling,
        benchmark_terminal_search,
        benchmark_terminal_resize,
        benchmark_terminal_hyperlink_detection,
        benchmark_terminal_memory_usage,
        benchmark_terminal_concurrent_operations
}

criterion_main!(terminal_benchmarks);
