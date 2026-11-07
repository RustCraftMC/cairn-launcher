//! Tests for crash analysis. Real logs and crash reports reside in `samples/` with player names replaced.

use std::path::{Path, PathBuf};

use super::report::Masker;
use super::*;

fn sample(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/crash/samples").join(name);
    collect::read_text(&path).unwrap()
}

fn causes(a: &CrashAnalysis) -> Vec<Cause> {
    a.findings.iter().map(|f| f.cause).collect()
}

fn details(a: &CrashAnalysis, cause: Cause) -> Vec<String> {
    a.findings.iter().find(|f| f.cause == cause).map(|f| f.details.clone()).unwrap_or_default()
}

fn game_log(text: &str) -> CrashLogs {
    CrashLogs { game_log: Some(text.to_string()), exit_code: Some(1), ..CrashLogs::default() }
}

fn crash_report(text: &str) -> CrashLogs {
    CrashLogs { crash_report: Some(text.to_string()), exit_code: Some(-1), ..CrashLogs::default() }
}

fn hs_err(text: &str) -> CrashLogs {
    CrashLogs { hs_err: Some(text.to_string()), exit_code: Some(1), ..CrashLogs::default() }
}

#[test]
fn fabric_incompatible_mods_from_gbk_log() {
    // Fabric writes localized errors into `latest.log` using the system code page.
    let log = sample("fabric-incompatible-gbk.log");
    assert!(log.contains("确定了一种可能的解决方法"));
    let a = analyze(&game_log(&log));
    assert_eq!(causes(&a), [Cause::ModIncompatible]);
    let d = details(&a, Cause::ModIncompatible);
    assert!(d[0].contains("'Fabric Loader' (fabricloader) 0.15.11 替换为 0.16.10"), "{d:?}");
    assert!(!d[0].contains("Some of your mods"));
    assert!(!d[0].contains("更多信息"));
}

#[test]
fn mixin_failure_names_the_mod() {
    let a = analyze(&crash_report(&sample("fabric-mixin.txt")));
    assert_eq!(causes(&a), [Cause::ModMixin]);
    assert_eq!(details(&a, Cause::ModMixin), ["anthropicclient"]);
    assert!(a.key_lines[0].starts_with("Description: Initializing game"));
    assert!(a.key_lines.iter().any(|l| l.contains("MixinApplyError")));
}

#[test]
fn mixin_failure_is_matched_to_a_mod_file() {
    let log = "[12:00:01] [main/ERROR]: Mixin apply for mod sodium failed sodium.mixins.json:features.render.MixinWorldRenderer from mod sodium -> net.minecraft.class_761: org.spongepowered.asm.mixin.injection.throwables.InvalidInjectionException\n\
               org.spongepowered.asm.mixin.injection.throwables.InvalidInjectionException: Critical injection failure\n";
    let mut logs = game_log(log);
    logs.mod_files = vec!["fabric-api-0.100.1+1.21.jar".into(), "sodium-fabric-0.5.11+mc1.21.jar".into()];
    let a = analyze(&logs);
    assert_eq!(details(&a, Cause::ModMixin), ["sodium-fabric-0.5.11+mc1.21.jar"]);
}

#[test]
fn suspected_mods_and_stack_keywords() {
    let report = sample("forge-stack-keyword.txt");
    let a = analyze(&crash_report(&report));
    assert_eq!(causes(&a), [Cause::ModSuspected]);
    assert_eq!(details(&a, Cause::ModSuspected), ["4690287328338162144@2@8.jar", "4690287328241054167@2@8.jar"]);

    // When only the initial stack trace is available, extracts the keyword from package names.
    let a = analyze(&crash_report(report.split("A detailed walkthrough").next().unwrap()));
    assert_eq!(causes(&a), [Cause::StackKeyword]);
    assert!(details(&a, Cause::StackKeyword).contains(&"heypixel".to_string()));
    assert!(a.key_lines.iter().any(|l| l.contains("java.lang.NullPointerException")));
}

#[test]
fn ticking_entity() {
    let a = analyze(&crash_report(&sample("forge-ticking-entity.txt")));
    assert_eq!(causes(&a), [Cause::EntityCrash]);
    assert_eq!(details(&a, Cause::EntityCrash), ["minecraft:player (10.44, -59.00, 9.51)"]);
    // Trailing Mixin annotations on stack trace lines are stripped.
    assert!(a.key_lines.iter().all(|l| !l.contains("{re:")));
}

#[test]
fn native_crash_in_hs_err() {
    let a = analyze(&hs_err(&sample("hs-err-native.log")));
    assert_eq!(causes(&a), [Cause::NativeCrash]);
    assert_eq!(details(&a, Cause::NativeCrash), ["[glfw.dll+0x101ab]"]);
    assert!(a.key_lines.iter().any(|l| l.contains("EXCEPTION_ACCESS_VIOLATION")));
}

#[test]
fn out_of_memory() {
    let log = "[14:02:11] [Render thread/INFO]: Loaded 1290 recipes\n\
               [14:05:42] [Server thread/ERROR]: Encountered an unexpected exception\n\
               java.lang.OutOfMemoryError: Java heap space\n\
               \tat java.util.Arrays.copyOf(Arrays.java:3512) ~[?:?]\n";
    assert_eq!(causes(&analyze(&game_log(log))), [Cause::OutOfMemory]);

    let jvm = "Error occurred during initialization of VM\nCould not reserve enough space for 4194304KB object heap\n";
    let logs = CrashLogs { output: jvm.lines().map(String::from).collect(), exit_code: Some(1), ..CrashLogs::default() };
    assert_eq!(causes(&analyze(&logs)), [Cause::OutOfMemory]);

    let hs = "# There is insufficient memory for the Java Runtime Environment to continue.\n# Native memory allocation (mmap) failed to map 262144 bytes for G1 virtual space\n# Possible reasons:\n#   The system is out of physical RAM or swap space\n";
    assert_eq!(causes(&analyze(&hs_err(hs))), [Cause::OutOfMemory]);
}

#[test]
fn java_32bit() {
    let out = vec!["Error occurred during initialization of VM".to_string(), "Could not reserve enough space for 1048576KB object heap".to_string()];
    let a = analyze(&CrashLogs { output: out, exit_code: Some(1), ..CrashLogs::default() });
    assert_eq!(causes(&a), [Cause::Java32Bit]);
}

#[test]
fn java_version() {
    let old = "Exception in thread \"main\" java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0), this version of the Java Runtime only recognizes class file versions up to 52.0\n\
               \tat java.lang.ClassLoader.defineClass1(Native Method)\n";
    let a = analyze(&CrashLogs { output: old.lines().map(String::from).collect(), exit_code: Some(1), ..CrashLogs::default() });
    assert_eq!(causes(&a), [Cause::JavaIncompatible]);
    assert_eq!(details(&a, Cause::JavaIncompatible), ["Java 21"]);

    let java11 = "java.lang.UnsupportedClassVersionError: com/example/Mod has been compiled by a more recent version of the Java Runtime (class file version 55.0), this version of the Java Runtime only recognizes class file versions up to 52.0\n";
    assert_eq!(causes(&analyze(&game_log(java11))), [Cause::ModNeedsJava11]);

    let too_new = "[main/ERROR] [cpw.mods.modlauncher.Launcher/MODLAUNCHER]: java.lang.reflect.InaccessibleObjectException: Unable to make field private final java.lang.Object jdk.internal.loader.URLClassPath.ucp accessible: because module java.base does not export jdk.internal.loader to unnamed module\n";
    assert_eq!(causes(&analyze(&game_log(too_new))), [Cause::JavaTooNew]);

    let asm = "[main/ERROR]: Unsupported class file major version 65\njava.lang.IllegalArgumentException: Unsupported class file major version 65\n";
    assert_eq!(causes(&analyze(&game_log(asm))), [Cause::JavaIncompatible]);
}

#[test]
fn graphics_driver() {
    let report = "---- Minecraft Crash Report ----\nDescription: Initializing game\n\norg.lwjgl.LWJGLException: Pixel format not accelerated\n\tat org.lwjgl.opengl.WindowsPeerInfo.nChoosePixelFormat(Native Method)\n";
    assert_eq!(causes(&analyze(&crash_report(report))), [Cause::GraphicsDriver]);

    let hs =
        "#  EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x00007ffb1a2c33d0, pid=1234, tid=5678\n#\n# Problematic frame:\n# C  [ig9icd64.dll+0x233d0]\n#\n";
    let a = analyze(&hs_err(hs));
    assert_eq!(causes(&a), [Cause::GraphicsDriver]);
    assert_eq!(details(&a, Cause::GraphicsDriver), ["Intel"]);

    let mut logs = hs_err(hs);
    logs.game_log = Some("[12:00:00] [Render thread/INFO]: Backend library: LWJGL version 3.3.1\n[12:00:00] [Render thread/INFO]: OpenGL: Intel(R) HD Graphics 620 GL version 4.6.0\n".into());
    assert_eq!(causes(&analyze(&logs)), [Cause::IntegratedGraphics]);

    let gl = "[main/ERROR]: The driver does not appear to support OpenGL\n";
    assert_eq!(causes(&analyze(&game_log(gl))), [Cause::OpenGlUnsupported]);
}

#[test]
fn missing_dependencies() {
    let log = "[22:10:31] [main/ERROR] [ne.mi.fm.lo.ModSorter/LOADING]: Missing or unsupported mandatory dependencies:\n\
               \tMod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'\n\
               \tMod ID: 'forge', Requested by: 'create', Expected range: '[47.1.3,)', Actual version: '47.0.19'\n\
               [22:10:31] [main/INFO] [ne.mi.fm.lo.ModLoader/LOADING]: Some mods failed to load\n";
    let a = analyze(&game_log(log));
    assert_eq!(causes(&a), [Cause::ModMissingDependency]);
    assert_eq!(
        details(&a, Cause::ModMissingDependency),
        [
            "Mod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'",
            "Mod ID: 'forge', Requested by: 'create', Expected range: '[47.1.3,)', Actual version: '47.0.19'",
        ]
    );
}

#[test]
fn duplicate_mods() {
    let forge = "[main/ERROR] [ne.mi.fm.lo.ModSorter/LOADING]: Found duplicate mods:\n\tMod ID: 'jei' from mod files: jei-1.20.1-forge-15.2.0.27.jar, jei-1.20.1-forge-15.3.0.4.jar\n";
    let a = analyze(&game_log(forge));
    assert_eq!(causes(&a), [Cause::ModDuplicate]);
    assert_eq!(details(&a, Cause::ModDuplicate), ["jei"]);

    let fabric = "net.fabricmc.loader.impl.discovery.ModResolutionException: Duplicate fabric mod ids found: sodium: C:\\Games\\.minecraft\\mods\\sodium-0.5.8.jar, C:\\Games\\.minecraft\\mods\\sodium-0.5.11.jar\n";
    let a = analyze(&game_log(fabric));
    assert_eq!(details(&a, Cause::ModDuplicate), ["sodium-0.5.8.jar", "sodium-0.5.11.jar"]);
}

#[test]
fn mod_crashed_resolved_through_crash_report_mod_list() {
    let report = "---- Minecraft Crash Report ----\n\
                  Description: Mod loading error has occurred\n\n\
                  java.lang.Exception: Mod Loading has failed\n\
                  \tat net.minecraftforge.logging.CrashReportExtender.dumpModLoadingCrashReport(CrashReportExtender.java:55)\n\n\
                  A detailed walkthrough of the error, its code path and all known details is as follows:\n\
                  ---------------------------------------------------------------------------------------\n\n\
                  -- MOD create --\n\
                  Details:\n\
                  \tCaused by 0: java.lang.NoClassDefFoundError: dev/engine_room/flywheel/api/visualization/VisualizationManager\n\
                  \tMod File: /C:/Games/.minecraft/mods/create-1.20.1-6.0.4.jar\n\
                  \tFailure message: Create (create) has failed to load correctly\n\
                  \t\tjava.lang.NoClassDefFoundError: dev/engine_room/flywheel/api/visualization/VisualizationManager\n\
                  \tMod Version: 6.0.4\n";
    let a = analyze(&crash_report(report));
    assert_eq!(causes(&a), [Cause::ModCrashed]);
    assert_eq!(details(&a, Cause::ModCrashed), ["create-1.20.1-6.0.4.jar"]);
}

#[test]
fn optifine_forge_conflict() {
    let log = "[Render thread/ERROR]: Exception caught during firing event: 'void net.minecraft.client.renderer.texture.SpriteContents.<init>(net.minecraft.resources.ResourceLocation, net.minecraft.client.resources.metadata.animation.FrameSize, com.mojang.blaze3d.platform.NativeImage, net.minecraft.server.packs.resources.ResourceMetadata)'\n\
               java.lang.NoSuchMethodError: 'void net.minecraft.client.renderer.texture.SpriteContents.<init>(net.minecraft.resources.ResourceLocation, net.minecraft.client.resources.metadata.animation.FrameSize, com.mojang.blaze3d.platform.NativeImage, net.minecraft.server.packs.resources.ResourceMetadata)'\n";
    assert_eq!(causes(&analyze(&game_log(log))), [Cause::OptiFineForge]);
}

#[test]
fn file_verification() {
    let log = "java.lang.SecurityException: class \"net.minecraftforge.common.ForgeHooks\"'s signer information does not match signer information of other classes in the same package\n";
    let a = analyze(&game_log(log));
    assert_eq!(causes(&a), [Cause::FileVerification]);
    assert_eq!(details(&a, Cause::FileVerification), ["net.minecraftforge.common.ForgeHooks"]);
}

#[test]
fn manual_close_and_debug_crash() {
    let logs = CrashLogs { game_log: Some("[12:00:00] [Render thread/INFO]: Stopping!\n".into()), exit_code: Some(EXIT_CONTROL_C), ..CrashLogs::default() };
    assert_eq!(causes(&analyze(&logs)), [Cause::ManualClose]);
    let logs = CrashLogs { exit_code: Some(EXIT_HUNG), ..CrashLogs::default() };
    assert_eq!(causes(&analyze(&logs)), [Cause::NotResponding]);

    let report = "---- Minecraft Crash Report ----\nDescription: Manually triggered debug crash\n\njava.lang.Throwable: Manually triggered debug crash\n\tat net.minecraft.client.KeyboardHandler.tick(KeyboardHandler.java:487)\n";
    assert_eq!(causes(&analyze(&crash_report(report))), [Cause::DebugCrash]);
}

#[test]
fn terminated_without_any_error() {
    let log = "[12:00:00] [Render thread/INFO]: Setting user: Steve\n[12:00:05] [Render thread/INFO]: Sound engine started\n[12:30:00] [Render thread/INFO]: Saving chunks for level 'ServerLevel[新的世界]'/minecraft:overworld\n";
    let a = analyze(&game_log(log));
    assert_eq!(causes(&a), [Cause::Terminated]);
}

#[test]
fn no_logs() {
    let a = analyze(&CrashLogs { exit_code: Some(1), ..CrashLogs::default() });
    assert_eq!(causes(&a), [Cause::NoLogs]);
    let a = analyze(&CrashLogs { exit_code: Some(EXIT_ACCESS_VIOLATION), ..CrashLogs::default() });
    assert_eq!(causes(&a), [Cause::NoLogs, Cause::NativeCrash]);
    assert_eq!(details(&a, Cause::NativeCrash), ["-1073741819 (0xC0000005)"]);
}

#[test]
fn short_output() {
    let logs =
        CrashLogs { output: vec!["错误: 找不到或无法加载主类 net.minecraft.client.main.Main".into()], exit_code: Some(1), ..CrashLogs::default() };
    let a = analyze(&logs);
    assert_eq!(causes(&a), [Cause::ShortOutput]);
}

#[test]
fn unknown_crash_falls_back_to_key_lines() {
    let output = [
        "[12:00:00] [Render thread/INFO]: Reloading ResourceManager: vanilla",
        "[12:00:09] [Render thread/ERROR]: Unreported exception thrown!",
        "java.lang.IllegalStateException: Rendersystem called from wrong thread",
        "\tat com.mojang.blaze3d.systems.RenderSystem.assertOnRenderThread(RenderSystem.java:124) ~[client-1.21.jar:?]",
        "\tat net.minecraft.client.renderer.GameRenderer.render(GameRenderer.java:870) ~[client-1.21.jar:?]",
        "[12:00:09] [Render thread/INFO]: Stopping!",
    ];
    let logs = CrashLogs { output: output.iter().map(|s| s.to_string()).collect(), exit_code: Some(-1), ..CrashLogs::default() };
    let a = analyze(&logs);
    assert!(a.findings.is_empty(), "{:?}", a.findings);
    assert_eq!(
        a.key_lines,
        [
            "java.lang.IllegalStateException: Rendersystem called from wrong thread",
            "    at com.mojang.blaze3d.systems.RenderSystem.assertOnRenderThread(RenderSystem.java:124)",
            "    at net.minecraft.client.renderer.GameRenderer.render(GameRenderer.java:870)",
        ]
    );
}

#[test]
fn masker_hides_token_and_user_name() {
    let m = Masker::with_user(vec!["eyJhbGciOiJIUzI1NiJ9.secret".into()], "Steve");
    let text = r#"java.exe -Djava.library.path=C:\Users\Steve\AppData\x --accessToken eyJhbGciOiJIUzI1NiJ9.secret --uuid 1 C:\Users\Steve2\a"#;
    let masked = m.apply(text);
    assert_eq!(masked, r#"java.exe -Djava.library.path=C:\Users\***\AppData\x --accessToken ******** --uuid 1 C:\Users\Steve2\a"#);
    // Tokens in arguments are masked even if not included in the secret list.
    assert_eq!(Masker::default().apply("--accessToken 0123456789abcdef --version 1.21"), "--accessToken ******** --version 1.21");
}

#[test]
fn launch_errors_are_explained() {
    let a = explain_error(&crate::Error::Java("没有找到满足「Java 21」的 64 位 Java,请安装或开启自动下载".into()));
    assert_eq!(causes(&a), [Cause::JavaNotFound]);
    let io = crate::Error::Io { path: PathBuf::from(r"D:\Games\.minecraft\libraries\a.jar"), source: std::io::Error::from_raw_os_error(32) };
    let a = explain_error(&io);
    assert_eq!(causes(&a), [Cause::FileAccessDenied]);
    assert_eq!(details(&a, Cause::FileAccessDenied), [r"D:\Games\.minecraft\libraries\a.jar"]);
    let a = explain_error(&crate::Error::HttpStatus { url: "https://bmclapi2.bangbang93.com/x.jar".into(), status: 404 });
    assert_eq!(details(&a, Cause::DownloadFailed), ["HTTP 404 · https://bmclapi2.bangbang93.com/x.jar"]);
    assert!(explain_error(&crate::Error::Invalid("别的".into())).findings.is_empty());
}

#[test]
fn collect_picks_files_written_after_launch() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp").join(format!("crash-collect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crash-reports")).unwrap();
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    std::fs::write(dir.join("crash-reports/crash-old-client.txt"), "old").unwrap();
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(1);
    // Move the old file timestamp back to ensure it predates launch despite timestamp resolution.
    let old = std::fs::File::options().write(true).open(dir.join("crash-reports/crash-old-client.txt")).unwrap();
    old.set_modified(since - std::time::Duration::from_secs(600)).unwrap();
    std::fs::write(dir.join("crash-reports/crash-new-client.txt"), "---- Minecraft Crash Report ----\nDescription: Manually triggered debug crash\n").unwrap();
    std::fs::write(dir.join("logs/latest.log"), "[12:00:00] [main/INFO]: hello\n").unwrap();
    std::fs::write(dir.join("hs_err_pid42.log"), "#  EXCEPTION_ACCESS_VIOLATION\n").unwrap();
    std::fs::write(dir.join("mods/Sodium-0.5.jar"), "").unwrap();
    std::fs::write(dir.join("mods/old.jar.disabled"), "").unwrap();

    let files = collect::collect(&dir, &[], since);
    let names: Vec<String> = files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["crash-new-client.txt", "hs_err_pid42.log", "latest.log"]);
    let logs = collect::to_logs(&files, Vec::new(), Some(-1), collect::mod_files(&dir.join("mods")));
    assert_eq!(logs.mod_files, ["Sodium-0.5.jar"]);
    assert_eq!(causes(&analyze(&logs)), [Cause::DebugCrash]);

    let zip_path = dir.join("report.zip");
    let entries = report::log_entries(&files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), &Masker::default());
    report::write_zip(&zip_path, &entries).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    assert_eq!(zip.len(), 3);
    assert!(zip.by_name("latest.log").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
