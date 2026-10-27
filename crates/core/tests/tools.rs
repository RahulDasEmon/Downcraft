use downcraft_core::{
    tools::{self, Busy, Tool, ToolBox},
    Manager,
};
use std::{sync::Arc, time::{Duration, Instant}};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("fc-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(unix)]
#[test]
fn locate_prefers_managed_and_remove_never_touches_system_copies() {
    let data = tmp("locate");
    let sys = tmp("syspath");
    std::fs::write(sys.join("deno"), "x").unwrap();
    std::env::set_var("PATH", &sys);

    // found on PATH: usable, but not managed
    let found = tools::locate(&data, Tool::Deno).unwrap();
    assert!(!found.managed);

    // a managed copy takes priority
    std::fs::create_dir_all(tools::tools_dir(&data)).unwrap();
    std::fs::write(tools::tools_dir(&data).join("deno"), "y").unwrap();
    assert!(tools::locate(&data, Tool::Deno).unwrap().managed);

    // removing deletes only the managed one
    tools::remove(&data, Tool::Deno).unwrap();
    assert!(!tools::tools_dir(&data).join("deno").exists());
    assert!(sys.join("deno").exists());
    assert!(!tools::locate(&data, Tool::Deno).unwrap().managed);
    // removing something that isn't there is fine
    tools::remove(&data, Tool::Ffmpeg).unwrap();
}

#[test]
fn every_tool_has_a_source_or_a_hint_on_this_system() {
    for t in tools::ALL {
        match t.source() {
            Some(s) => assert!(s.url.starts_with("https://github.com/"), "{}", s.url),
            None => assert!(!t.manual_hint().is_empty()),
        }
    }
}

fn wait_idle(tb: &Arc<ToolBox>, secs: u64) {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if !tb.is_busy() {
            return;
        }
        assert!(Instant::now() < end, "timed out");
    }
}

/// Real downloads from GitHub (~250 MB): `cargo test -p downcraft-core --test tools -- --ignored`
#[test]
#[ignore]
fn install_and_remove_real_tools() {
    let m = Manager::with_dir(tmp("real"));
    let tb = ToolBox::new(m.clone());
    wait_idle(&tb, 60);
    for t in tools::ALL {
        if t.source().is_none() {
            continue;
        }
        tb.install(t);
        wait_idle(&tb, 900);
        let e = tb.snapshot().into_iter().find(|(x, _)| *x == t).unwrap().1;
        assert_eq!(e.busy, Busy::Idle);
        assert!(e.message.as_ref().map_or(true, |m| !m.0), "{:?}: {:?}", t, e.message);
        let inst = e.installed.clone().unwrap_or_else(|| panic!("{t:?} not installed: {:?}", e.message));
        assert!(inst.managed, "{:?}", inst);
        println!("{:?} -> {} {:?}", t, inst.path.display(), e.version);
        assert!(e.version.is_some());
    }
    // yt-dlp must be usable by DownCraft's finder
    assert!(downcraft_core::ytdlp::find(m.data_dir()).is_some());
    for t in tools::ALL {
        tb.remove(t);
        wait_idle(&tb, 60);
        let e = tb.snapshot().into_iter().find(|(x, _)| *x == t).unwrap().1;
        assert!(e.installed.as_ref().map_or(true, |i| !i.managed));
    }
}
