use std::sync::{Arc, Once};

use super::*;

#[derive(Clone)]
pub struct Exit {
    flag:	Arc<()>,
}

impl Exit {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(())
        }
    }

    pub fn is_running(&self) -> bool {
        Arc::strong_count(&self.flag) > 1
    }

    pub fn do_exit(self) {
    }
}

struct TestEnvironment {
    dir:	tempfile::TempDir,
    bindir:	PathBuf,
    tmpdir:	PathBuf,
    rundir:	PathBuf,
}

impl TestEnvironment {
    pub fn new() -> Self {
        let d = tempfile::TempDir::new().unwrap();

        Self {
            bindir:	d.path().join("bin"),
            tmpdir:	d.path().join("tmp"),
            rundir:	d.path().join("run"),
            dir:	d,
        }.init()
    }

    fn init(self) -> Self {
        std::fs::create_dir(&self.bindir).unwrap();
        std::fs::create_dir(&self.tmpdir).unwrap();
        std::fs::create_dir(&self.rundir).unwrap();

        self.mkdir("a");
        self.mkdir("b/c/d");

        self.touch("a/pre-a");
        self.touch("b/pre-b");
        self.touch("b/c/pre-c");
        self.touch("b/c/d/pre-d");

        self
    }

    pub fn mkdir<P: AsRef<Path>>(&self, p: P) {
        let p = p.as_ref();

        std::fs::create_dir_all(self.tmpdir.join(p)).unwrap();
    }

    pub fn touch<P: AsRef<Path>>(&self, p: P) {
        let p = p.as_ref();

        std::fs::File::options()
            .create(true)
            .append(true)
            .open(self.tmpdir.join(p))
            .unwrap();
    }

    #[allow(unused)]
    pub fn touch_run<P: AsRef<Path>>(&self, p: P) {
        let p = p.as_ref();

        std::fs::File::options()
            .create(true)
            .append(true)
            .open(self.rundir.join(p))
            .unwrap();
    }

    pub fn unlink_run<P: AsRef<Path>>(&self, p: P) {
        let _ = std::fs::remove_file(self.rundir.join(p));
    }

    pub fn unlink<P: AsRef<Path>>(&self, p: P) {
        let _ = std::fs::remove_file(self.tmpdir.join(p));
    }

    pub fn path<P: AsRef<Path>>(&self, p: P) -> PathBuf {
        self.tmpdir.join(p)
    }

    pub fn script_dir() -> PathBuf {
        let p: &Path = env!("CARGO_MANIFEST_DIR").as_ref();
        p.join("tests")
    }
}

struct Cli {
    args: Vec<OsString>,
}

impl Cli {
    pub fn new() -> Self {
        Self { args: Vec::new() }
    }

    pub fn arg<S: Into<OsString>>(mut self, s: S) -> Self {
        self.args.push(s.into());
        self
    }
}

fn delay(cnt: u64) {
    std::thread::sleep(Duration::from_millis(100 * cnt));
}

fn exists<P: AsRef<Path>>(p: P) -> bool {
    std::fs::exists(p.as_ref()).unwrap()
}

fn init_tests() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        env_logger::init();
    });
}

#[test]
fn test_00() {
    init_tests();

    let env = TestEnvironment::new();
    let cli = Cli::new()
        .arg("wait-file")
        .arg("--watch")
        .arg(env.path("a/stamp-a"))
        .arg("--watch")
        .arg(env.path("b/c/d/file"))
        .arg("--build")
        .arg(format!("{:?} {:?}", TestEnvironment::script_dir().join("build"), env.dir.path()))
        .arg("--settle-time")
        .arg("0")
        .arg("--")
        .arg(TestEnvironment::script_dir().join("test"))
        .arg(env.dir.path())
        .arg(env.path("b/c/d/file"));

    println!("cli={:?}", cli.args);

    let args = Args::parse_from(cli.args);

    std::thread::scope(|s| {
        let exit = Exit::new();

        let h = s.spawn({
            let exit = exit.clone();
            move || {
                println!("running test_00");
                args.run(exit)
            }
        });

        info!("A test_00 starting");

        while !exists(env.rundir.join("build-1")) {
            delay(1);
        }

        info!("B test_00 starting");

        delay(1);

        assert!(!h.is_finished());
        assert!(!exists(env.rundir.join("test-run")));

        env.touch("b/c/d/file");

        delay(1);
        assert!(!h.is_finished());
        assert!(!exists(env.rundir.join("test-run")));

        env.touch("a/stamp-a");

        delay(5);

        assert!(!h.is_finished());
        assert!(exists(env.rundir.join("build-run")));
        assert!(exists(env.rundir.join("test-run")));
        assert!(exists(env.rundir.join("test-1")));
        assert!(exists(env.rundir.join("test-exists-1")));

        env.unlink_run("build-run");
        env.unlink_run("test-run");
        env.unlink("a/stamp-a");

        delay(5);

        assert!(!h.is_finished());
        assert!(exists(env.rundir.join("build-run")));
        assert!(!exists(env.rundir.join("test-run")));

        env.touch("a/stamp-a");
        delay(5);

        assert!(!h.is_finished());
        assert!(exists(env.rundir.join("build-run")));
        assert!(exists(env.rundir.join("test-run")));

        assert!(!h.is_finished());

        info!("test_00: waiting for exit");

        exit.do_exit();
        env.touch("a/stamp-a");
        h.join().unwrap();
    });
}

#[test]
fn test_01() {
    init_tests();

    let env = TestEnvironment::new();
    let cli = Cli::new()
        .arg("wait-file")
        .arg("--watch")
        .arg(env.path("b/c/d/file-0"))
        .arg("--watch")
        .arg(env.path("b/c/d/file-1"))
        .arg("--build")
        .arg(format!("{:?} {:?}", TestEnvironment::script_dir().join("build"), env.dir.path()))
        .arg("--settle-time")
        .arg("0")
        .arg("--")
        .arg(TestEnvironment::script_dir().join("test"))
        .arg(env.dir.path())
        .arg(env.path("b/c/d/file-0"));

    println!("cli={:?}", cli.args);

    let args = Args::parse_from(cli.args);

    std::thread::scope(|s| {
        let exit = Exit::new();

        let h = s.spawn({
            let exit = exit.clone();
            move || {
                println!("running test_01");
                args.run(exit)
            }
        });

        info!("A test_01 starting");

        while !exists(env.rundir.join("build-run")) {
            delay(1);
        }

        info!("B test_01 starting");

        delay(1);

        assert!(!h.is_finished());
        assert!(!exists(env.rundir.join("test-run")));

        env.unlink_run("build-run");

        env.touch("b/c/d/file-1");
        delay(5);

        assert!(!h.is_finished());
        assert!(exists(env.rundir.join("build-run")));
        assert!(!exists(env.rundir.join("test-run")));

        env.touch("b/c/d/file-0");
        delay(5);

        assert!(!h.is_finished());
        assert!(exists(env.rundir.join("test-run")));

        info!("test_01: waiting for exit");

        exit.do_exit();
        env.touch("b/c/d/file-0");
        h.join().unwrap();

    });
}
