//! Game crash analysis: pattern matching against crash reports, game logs, and JVM error files.
//! Priorities and rule sets classify root causes from key log patterns.
//! Lookarounds are rewritten as capture groups, since the regex crate lacks them.

use std::collections::HashSet;

use regex_lite::Regex;

use super::{Cause, CrashAnalysis, CrashLogs, EXIT_ACCESS_VIOLATION, EXIT_CONTROL_C, EXIT_HUNG, EXIT_STACK_BUFFER_OVERRUN, Finding, format_exit_code};

pub(super) struct Analyzer<'a> {
    /// Combined game output and latest log contents.
    mc: Option<String>,
    debug: Option<String>,
    hs: Option<String>,
    crash: Option<String>,
    all: String,
    exit_code: Option<i32>,
    mod_files: &'a [String],
    findings: Vec<Finding>,
}

impl<'a> Analyzer<'a> {
    pub(super) fn new(logs: &'a CrashLogs) -> Self {
        let mut mc = String::new();
        if !logs.output.is_empty() {
            let lines: Vec<&str> = logs.output.iter().map(String::as_str).collect();
            mc.push_str(&head_tail(&lines, 0, 500));
        }
        if let Some(log) = &logs.game_log {
            mc.push_str(&head_tail(&log.lines().collect::<Vec<_>>(), 1500, 500));
        }
        let debug = logs.debug_log.as_deref().map(|d| head_tail(&d.lines().collect::<Vec<_>>(), 1000, 0)).filter(|d| !d.is_empty());
        let mc = match (mc.is_empty(), &debug) {
            (false, _) => Some(mc),
            (true, Some(d)) => Some(d.clone()),
            (true, None) => None,
        };
        let crash = logs.crash_report.as_deref().map(|c| head_tail(&c.lines().collect::<Vec<_>>(), 300, 700)).filter(|c| !c.is_empty());
        let hs = logs.hs_err.as_deref().map(|h| head_tail(&h.lines().collect::<Vec<_>>(), 200, 100)).filter(|h| !h.is_empty());
        let all = [&mc, &debug, &hs, &crash].into_iter().flatten().map(String::as_str).collect::<Vec<_>>().concat();
        Analyzer { mc, debug, hs, crash, all, exit_code: logs.exit_code, mod_files: &logs.mod_files, findings: Vec::new() }
    }

    pub(super) fn run(mut self) -> CrashAnalysis {
        let key_lines = self.key_lines();
        self.analyze_exit_code();
        if self.findings.is_empty() {
            self.crit1();
        }
        if self.findings.is_empty() {
            self.crit2();
        }
        if self.findings.is_empty() && !self.analyze_stack() {
            self.crit3();
        }
        if self.findings.is_empty() {
            self.analyze_native();
        }
        CrashAnalysis { findings: self.findings, key_lines }
    }

    fn add(&mut self, cause: Cause, details: impl IntoIterator<Item = String>) {
        let details = details.into_iter().map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
        match self.findings.iter_mut().find(|f| f.cause == cause) {
            Some(f) => {
                for d in details {
                    if !f.details.contains(&d) {
                        f.details.push(d);
                    }
                }
            }
            None => {
                let mut list: Vec<String> = Vec::new();
                for d in details {
                    if !list.contains(&d) {
                        list.push(d);
                    }
                }
                self.findings.push(Finding { cause, details: list });
            }
        }
    }

    fn has(&self, cause: Cause) -> bool {
        self.findings.iter().any(|f| f.cause == cause)
    }

    /// Exit codes that explain the problem by themselves: the user closed the game,
    /// or the game was killed after it stopped responding.
    fn analyze_exit_code(&mut self) {
        match self.exit_code {
            Some(EXIT_CONTROL_C) => self.add(Cause::ManualClose, []),
            Some(EXIT_HUNG) => self.add(Cause::NotResponding, []),
            _ => {}
        }
    }

    /// Exact log matching, highest priority.
    fn crit1(&mut self) {
        if self.mc.is_none() && self.hs.is_none() && self.crash.is_none() {
            self.add(Cause::NoLogs, []);
            if matches!(self.exit_code, Some(EXIT_ACCESS_VIOLATION | EXIT_STACK_BUFFER_OVERRUN)) {
                self.add(Cause::NativeCrash, self.exit_code.map(format_exit_code));
            }
            return;
        }

        if let Some(crash) = self.crash.clone() {
            if crash.contains("Unable to make protected final java.lang.Class java.lang.ClassLoader.defineClass") {
                self.add(Cause::JavaTooNew, []);
            }
            if crash.contains("Failed loading config file ") {
                self.add_mod_config(&crash);
            }
        }

        if let Some(mc) = self.mc.clone() {
            let has = |s: &str| mc.contains(s);
            if has("Unrecognized option:") {
                self.add(Cause::JvmArgsInvalid, []);
            }
            if has("Found multiple arguments for option fml.forgeVersion, but you asked for only one") {
                self.add(Cause::MultipleForge, []);
            }
            if has("The driver does not appear to support OpenGL") {
                self.add(Cause::OpenGlUnsupported, []);
            }
            if has("java.lang.ClassCastException: java.base/jdk") || has("java.lang.ClassCastException: class jdk.") {
                self.add(Cause::UsingJdk, []);
            }
            const OPTIFINE_FORGE: [&str; 6] = [
                "java.lang.NoSuchMethodError: 'void net.minecraft.client.renderer.texture.SpriteContents.<init>",
                "java.lang.NoSuchMethodError: 'java.lang.String com.mojang.blaze3d.systems.RenderSystem.getBackendDescription",
                "java.lang.NoSuchMethodError: 'void net.minecraft.client.renderer.block.model.BakedQuad.<init>",
                "java.lang.NoSuchMethodError: 'void net.minecraftforge.client.gui.overlay.ForgeGui.renderSelectedItemName",
                "java.lang.NoSuchMethodError: 'void net.minecraft.server.level.DistanceManager",
                "java.lang.NoSuchMethodError: 'net.minecraft.network.chat.FormattedText net.minecraft.client.gui.Font.ellipsize",
            ];
            if OPTIFINE_FORGE.iter().any(|s| has(s)) {
                self.add(Cause::OptiFineForge, []);
            }
            if has("Open J9 is not supported") || has("OpenJ9 is incompatible") || has(".J9VMInternals.") {
                self.add(Cause::OpenJ9, []);
            }
            if has("java.lang.NoSuchFieldException: ucp")
                || has("because module java.base does not export")
                || has("java.lang.ClassNotFoundException: jdk.nashorn.api.scripting.NashornScriptEngineFactory")
                || has("java.lang.ClassNotFoundException: java.lang.invoke.LambdaMetafactory")
            {
                self.add(Cause::JavaTooNew, []);
            }
            if has("The directories below appear to be extracted jar files. Fix this before you continue.")
                || has("Extracted mod jars found, loading will NOT continue")
            {
                self.add(Cause::ModExtracted, []);
            }
            if has("java.lang.ClassNotFoundException: org.spongepowered.asm.launch.MixinTweaker") {
                self.add(Cause::MixinBootstrapMissing, []);
            }
            if has("Couldn't set pixel format") {
                self.add(self.graphics_cause(), []);
            }
            if has("java.lang.OutOfMemoryError") || has("an out of memory error") {
                self.add(Cause::OutOfMemory, []);
            }
            if has("java.lang.RuntimeException: Shaders Mod detected. Please remove it, OptiFine has built-in support for shaders.") {
                self.add(Cause::ShadersModOptiFine, []);
            }
            if has("java.lang.NoSuchMethodError: sun.security.util.ManifestEntryVerifier")
                || has("java.lang.NoSuchMethodError: 'void sun.security.util.ManifestEntryVerifier")
            {
                self.add(Cause::OldForgeNewJava, []);
            }
            if has("1282: Invalid operation") {
                self.add(Cause::OpenGl1282, []);
            }
            if has("signer information does not match signer information of other classes in the same package") {
                self.add(Cause::FileVerification, seek(&mc, r#"class "([^']+)"'s signer information"#));
            }
            if has("Maybe try a lower resolution resourcepack?") {
                self.add(Cause::TextureTooLarge, []);
            }
            if has("java.lang.NoSuchMethodError: net.minecraft.world.server.ChunkManager$ProxyTicketManager.shouldForceTicks(J)Z") && has("OptiFine") {
                self.add(Cause::OptiFineWorldLoad, []);
            }
            if has("Unsupported class file major version") || has("Unsupported major.minor version") {
                self.add(Cause::JavaIncompatible, []);
            }
            if has("com.electronwill.nightconfig.core.io.ParsingException: Not enough data available") && !self.has(Cause::ModConfig) {
                self.add(Cause::NightConfig, []);
            }
            if has("Cannot find launch target fmlclient, unable to launch")
                || (has("Invalid paths argument, contained no existing paths") && has(r"libraries\net\minecraftforge\fmlcore"))
            {
                self.add(Cause::ForgeIncomplete, []);
            }
            if has("Invalid module name: '' is not a Java identifier") {
                self.add(Cause::ModNameSpecialChars, []);
            }
            if has(
                "has been compiled by a more recent version of the Java Runtime (class file version 55.0), this version of the Java Runtime only recognizes class file versions up to",
            ) || has(
                "java.lang.RuntimeException: java.lang.NoSuchMethodException: no such method: sun.misc.Unsafe.defineAnonymousClass(Class,byte[],Object[])Class/invokeVirtual",
            ) || has(
                "java.lang.IllegalArgumentException: The requested compatibility level JAVA_11 could not be set. Level is not supported by the active JRE or ASM version",
            ) {
                self.add(Cause::ModNeedsJava11, []);
            }
            // The original only recognizes the Java 11 message.
            // Other versions are derived from the class file version.
            if has("has been compiled by a more recent version of the Java Runtime") && !self.has(Cause::ModNeedsJava11) {
                let needed =
                    seek(&mc, r"class file version (\d+)\.0\)").and_then(|v| v.parse::<u32>().ok()).filter(|v| *v > 44).map(|v| format!("Java {}", v - 44));
                self.add(Cause::JavaIncompatible, needed);
            }
            if has("Invalid maximum heap size") {
                self.add(Cause::Java32Bit, []);
            }
            if has("Could not reserve enough space") {
                self.add(if has("for 1048576KB object heap") { Cause::Java32Bit } else { Cause::OutOfMemory }, []);
            }
            if has("Caught exception from ") {
                let names = self.try_mod_name(seek(&mc, r"Caught exception from ([^\n]+)").unwrap_or_default().trim());
                self.add(Cause::ModCrashed, names);
            }
            if has("DuplicateModsFoundException") {
                self.add(Cause::ModDuplicate, search(&mc, r"(?i)\n\t\w+ : [A-Z]:[^\n]*[/\\]([^/\\\n]+?\.jar)"));
            }
            if has("Found a duplicate mod") {
                let line = seek_all(&mc, r"Found a duplicate mod[^\n]+").unwrap_or_default();
                self.add(Cause::ModDuplicate, search_all(&line, r"(?i)[^\\/]+\.jar"));
            }
            if has("Found duplicate mods") {
                self.add(Cause::ModDuplicate, search(&mc, r"Mod ID: '(\w+?)' from mod files:"));
            }
            if has("ModResolutionException: Duplicate") {
                let line = seek_all(&mc, r"ModResolutionException: Duplicate[^\n]+").unwrap_or_default();
                self.add(Cause::ModDuplicate, search_all(&line, r"(?i)[^\\/]+\.jar"));
            }
            if has("Incompatible mods found!") {
                let raw = seek(&mc, r"Incompatible mods found![\s\S]*?: ([\s\S]+?)\tat ").unwrap_or_default();
                let raw = before_first(&raw, "更多信息：").replace("Some of your mods are incompatible with the game or each other!", "");
                self.add(Cause::ModIncompatible, [raw.trim().to_string()]);
            }
            if has("Missing or unsupported mandatory dependencies:") {
                self.add(Cause::ModMissingDependency, indented_after(&mc, "Missing or unsupported mandatory dependencies:"));
            }
        }

        if let Some(hs) = self.hs.clone() {
            if hs.contains("The system is out of physical RAM or swap space") || hs.contains("Out of Memory Error") {
                self.add(Cause::OutOfMemory, []);
            }
            if hs.contains("EXCEPTION_ACCESS_VIOLATION") {
                for (frame, vendor) in [("# C  [ig", "Intel"), ("# C  [atio", "AMD"), ("# C  [nvoglv", "NVIDIA")] {
                    if hs.contains(frame) {
                        self.add(self.graphics_cause(), [vendor.to_string()]);
                    }
                }
            }
        }

        if let Some(crash) = self.crash.clone() {
            let has = |s: &str| crash.contains(s);
            if has("maximum id range exceeded") {
                self.add(Cause::IdLimit, []);
            }
            if has("java.lang.OutOfMemoryError") {
                self.add(Cause::OutOfMemory, []);
            }
            if has("Pixel format not accelerated") {
                self.add(self.graphics_cause(), []);
            }
            if has("Manually triggered debug crash") {
                self.add(Cause::DebugCrash, []);
            }
            if has("has mods that were not found")
                && Regex::new(r"The Mod File [^\n]+optifine\\OptiFine[^\n]+ has mods that were not found").unwrap().is_match(&crash)
            {
                self.add(Cause::OptiFineForge, []);
            }
            if has("-- MOD ") {
                let section = between(&crash, "-- MOD ", "Failure message:");
                if section.to_ascii_lowercase().contains(".jar") {
                    // The log line may carry a full path, so only the file name is kept.
                    let file = seek(section, r"Mod File: ([^\n]+)").map(|f| f.trim().rsplit(['/', '\\']).next().unwrap_or_default().to_string());
                    self.add(Cause::ModCrashed, file);
                } else {
                    let msg = seek(&crash, r"Failure message: ([\s\S]+?)\tMod").unwrap_or_default().replace('\t', " ");
                    self.add(Cause::LoaderError, [msg]);
                }
            }
            if has("Multiple entries with same key: ") {
                let names = self.try_mod_name(seek(&crash, r"Multiple entries with same key: ([^=]+)").unwrap_or_default().trim());
                self.add(Cause::ModCrashed, names);
            }
            if has("LoaderExceptionModCrash: Caught exception from ") {
                let names = self.try_mod_name(seek(&crash, r"LoaderExceptionModCrash: Caught exception from ([^\n]+)").unwrap_or_default().trim());
                self.add(Cause::ModCrashed, names);
            }
            if has("Failed loading config file ") {
                self.add_mod_config(&crash);
            }
        }
    }

    fn add_mod_config(&mut self, crash: &str) {
        let modid = seek(crash, r"Failed loading config file .+ for modid ([^\n]+)").unwrap_or_default();
        let name = self.try_mod_name(modid.trim()).into_iter().next().unwrap_or_default();
        let file = seek(crash, r"Failed loading config file (.+) of type").unwrap_or_default();
        self.add(Cause::ModConfig, [name, file]);
    }

    /// Graphics driver problem.
    /// If the log mentions an Intel integrated GPU, it suggests a discrete GPU.
    fn graphics_cause(&self) -> Cause {
        if self.all.to_lowercase().contains("hd graphics ") { Cause::IntegratedGraphics } else { Cause::GraphicsDriver }
    }

    /// Exact log matches. Lower priority than `crit1`, higher than stack analysis.
    fn crit2(&mut self) {
        if let Some(mc) = self.mc.clone() {
            let is_mixin = self.analyze_mixin(&mc);
            if mc.contains("An exception was thrown, the game will display an error screen and halt.") {
                let msg = seek(&mc, r"the game will display an error screen and halt\.[\n\r]+[^\n]+?Exception: ([\s\S]+?)\n\tat");
                self.add(Cause::ForgeError, msg);
            }
            for marker in [
                "A potential solution has been determined:",
                "A potential solution has been determined, this may resolve your problem:",
                "确定了一种可能的解决方法，这样做可能会解决你的问题：",
            ] {
                if mc.contains(marker) {
                    let lines = solution_lines(&mc, marker);
                    self.add(Cause::FabricSolution, (!lines.is_empty()).then(|| lines.join("\n")));
                }
            }
            // OptiFabric Mixin failures also contain this text.
            // Do not read them as Fabric load failures.
            if !is_mixin && mc.contains("due to errors, provided by ") {
                let names = self.try_mod_name(seek(&mc, r"due to errors, provided by '([^']+)").unwrap_or_default().trim());
                self.add(Cause::ModCrashed, names);
            }
        }
        if let Some(crash) = self.crash.clone() {
            self.analyze_mixin(&crash);
            if crash.contains("Suspected Mod") {
                let raw = between(&crash, "Suspected Mod", "Stacktrace");
                if !raw.starts_with("s: None") {
                    let suspects = search(raw, r"\n\t[^(\t]+\(([^)\n]+)");
                    if !suspects.is_empty() {
                        let names = self.analyze_mod_name(&suspects).unwrap_or(suspects);
                        self.add(Cause::ModSuspected, names);
                    }
                }
            }
        }
    }

    /// Mixin injection failure. Names the mod when possible.
    /// Returns whether the crash is a Mixin failure.
    fn analyze_mixin(&mut self, text: &str) -> bool {
        let is_mixin = [
            "Mixin prepare failed ",
            "Mixin apply failed ",
            "MixinApplyError",
            "MixinTransformerError",
            "mixin.injection.throwables.",
            ".json] FAILED during )",
        ]
        .iter()
        .any(|s| text.contains(s));
        if !is_mixin {
            return false;
        }
        let name = seek(text, r"from mod ([^./ ]+)\] from").or_else(|| seek(text, r"for mod ([^./ ]+) failed"));
        if let Some(name) = name {
            let names = self.try_mod_name(name.trim());
            self.add(Cause::ModMixin, names);
            return true;
        }
        // Without a mod name, use the Mixin config file name.
        // For xxx.mixins.json, xxx is usually the mod ID.
        let json = Regex::new(r"[ \[{(]([^ \[{(\t]+\.[^ \t]+)\.json").unwrap();
        for line in text.lines() {
            let head = before_first(line, "\t");
            if let Some(c) = json.captures(head).filter(|c| c.get(0).is_some_and(|m| m.start() > 0)) {
                let name = c[1].replace("mixins", "mixin").replace(".mixin", "").replace("mixin.", "");
                let names = self.try_mod_name(&name);
                self.add(Cause::ModMixin, names);
                return true;
            }
        }
        self.add(Cause::ModMixin, []);
        true
    }

    /// Stack analysis. With a mod loader installed, looks for suspicious package names.
    /// Returns `true` when one is found.
    fn analyze_stack(&mut self) -> bool {
        if !["orge", "abric", "uilt", "iteloader"].iter().any(|s| self.all.contains(s)) {
            return false;
        }
        let mut keywords = Vec::new();
        if let Some(crash) = &self.crash {
            keywords.extend(stack_keywords(before_first(crash, "System Details")));
        }
        if let Some(mc) = &self.mc {
            let lines: Vec<&str> = mc.lines().collect();
            let mut fatals: Vec<String> = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                if let Some(p) = line.find("/FATAL] ")
                    && lines.get(i + 1).is_some_and(|next| next.starts_with('['))
                {
                    fatals.push(line[p..].to_string());
                }
            }
            if mc.contains("Unreported exception thrown!") {
                fatals.push(between(mc, "Unreported exception thrown!", "at oolloo.jlw.Wrapper").to_string());
            }
            for f in &fatals {
                keywords.extend(stack_keywords(f));
            }
        }
        if let Some(hs) = &self.hs {
            keywords.extend(stack_keywords(between(hs, "T H R E A D", "Registers:")));
        }
        dedup(&mut keywords);
        if keywords.is_empty() {
            return false;
        }
        match self.analyze_mod_name(&keywords) {
            Some(names) => self.add(Cause::ModSuspected, names),
            None => self.add(Cause::StackKeyword, keywords),
        }
        true
    }

    /// Exact log matches. Lower priority than stack analysis.
    fn crit3(&mut self) {
        if let Some(mc) = self.mc.clone() {
            if !(mc.contains("at net.") || mc.contains("INFO]")) && self.hs.is_none() && self.crash.is_none() && mc.chars().count() < 100 {
                self.add(Cause::ShortOutput, [mc.clone()]);
            }
            if mc.contains("Mod resolution failed") {
                self.add(Cause::LoaderError, []);
            }
            // A Mixin failure also makes many mods fail to create instances.
            // So this check has low priority.
            if mc.contains("Failed to create mod instance.") {
                let id =
                    seek(&mc, r"Failed to create mod instance\. ModID: ([^,]+)").or_else(|| seek(&mc, r"Failed to create mod instance\. ModId ([^\n]+) for "));
                let names = self.try_mod_name(id.unwrap_or_default().trim());
                self.add(Cause::ModInitFailed, names);
            }
        }
        if let Some(crash) = self.crash.clone() {
            if crash.contains("\tBlock location: World: ") {
                let block = seek(&crash, r"\tBlock: Block\{([^}]+)").unwrap_or_default();
                let at = seek(&crash, r"\tBlock location: World: (\([^)]+\))").unwrap_or_default();
                self.add(Cause::BlockCrash, [format!("{block} {at}")]);
            }
            if crash.contains("\tEntity's Exact location: ") {
                let kind = seek(&crash, r"\tEntity Type: ([^\n]+) \(").unwrap_or_default();
                let at = seek(&crash, r"\tEntity's Exact location: ([^\n]+)").unwrap_or_default();
                self.add(Cause::EntityCrash, [format!("{kind} ({})", at.trim())]);
            }
        }
    }

    /// Fallback when nothing else matched.
    /// Looks at the JVM crash log and the exit code.
    fn analyze_native(&mut self) {
        if let Some(hs) = &self.hs
            && hs.contains("EXCEPTION_ACCESS_VIOLATION")
        {
            let frame = seek(between(hs, "# Problematic frame:", "#\n"), r"# [A-Za-z]  (\S[^\n]*)");
            self.add(Cause::NativeCrash, frame);
            return;
        }
        let Some(code) = self.exit_code else { return };
        if matches!(code, EXIT_ACCESS_VIOLATION | EXIT_STACK_BUFFER_OVERRUN) {
            self.add(Cause::NativeCrash, [format_exit_code(code)]);
            return;
        }
        // Exit code 1 with no crash report and no error at the end of the log usually
        // means the process was killed outright.
        if code == 1 && self.crash.is_none() && self.hs.is_none() {
            let quiet = self.mc.as_deref().is_some_and(|mc| {
                let lines: Vec<&str> = mc.lines().collect();
                let tail = &lines[lines.len().saturating_sub(40)..];
                !tail.iter().any(|l| ["Exception", "Error", "FATAL", "\tat ", "Caused by"].iter().any(|k| l.contains(k)))
            });
            if quiet {
                self.add(Cause::Terminated, []);
            }
        }
    }

    /// Looks up mod file names for a keyword.
    /// Returns the keyword itself when nothing is found.
    fn try_mod_name(&self, keyword: &str) -> Vec<String> {
        if keyword.is_empty() {
            return Vec::new();
        }
        let raw = vec![keyword.to_string()];
        self.analyze_mod_name(&raw).unwrap_or(raw)
    }

    /// Finds the actual mod file names from keywords, usually a mod ID or a package segment.
    /// Returns `None` when nothing is found.
    fn analyze_mod_name(&self, keywords: &[String]) -> Option<Vec<String>> {
        let keywords: Vec<String> =
            keywords.iter().flat_map(|k| k.split('(').map(|s| s.trim_matches([' ', ')']).to_string()).collect::<Vec<_>>()).filter(|k| !k.is_empty()).collect();
        let mut names: Vec<String> = Vec::new();

        // The mod list is at the end of the crash report, after the walkthrough header.
        if let Some(crash) = &self.crash
            && crash.contains("A detailed walkthrough of the error")
        {
            let mut details = crash.replace("A detailed walkthrough of the error", "¨");
            let fabric = details.contains("Fabric Mods");
            if fabric {
                details = details.replace("Fabric Mods", "¨");
            }
            let details = details.rsplit('¨').next().unwrap_or("");
            let fabric_builtin = Regex::new(r"\t\tfabric[\w-]*: Fabric").unwrap();
            // Unlike the original, stack lines are skipped here.
            // A Mixin stack line has one .jar too, so it matches before the mod list,
            // but it yields no file name.
            let mod_lines: Vec<&str> = details
                .split('\n')
                .filter(|line| !line.trim_start().starts_with("at "))
                .filter(|line| {
                    (line.to_ascii_lowercase().contains(".jar") && line.matches(".jar").count() == 1)
                        || (fabric && line.starts_with("\t\t") && !fabric_builtin.is_match(line))
                })
                .collect();
            let mut hits: Vec<&str> = Vec::new();
            for kw in &keywords {
                let kw = kw.to_lowercase().replace('_', "");
                for line in &mod_lines {
                    let low = line.to_lowercase().replace('_', "");
                    if !low.contains(&kw) {
                        continue;
                    }
                    if low.contains("minecraft.jar") || low.contains(" forge-") || low.contains(" mixin-") {
                        continue;
                    }
                    hits.push(line.trim_matches(['\r', '\n']));
                    break;
                }
            }
            dedup(&mut hits);
            let forge_name = Regex::new(r"(?i)\(([^\t]+\.jar)\)|(?:\t\t|\| )([^\t|]+\.jar)").unwrap();
            for line in hits {
                let name = if fabric {
                    line.split_once(": ").and_then(|(_, rest)| rest.rsplit_once(' ').map(|(n, _)| n.to_string()))
                } else {
                    forge_name.captures(line).and_then(|c| c.get(1).or_else(|| c.get(2))).map(|m| m.as_str().to_string())
                };
                names.extend(name);
            }
        }

        // debug.log has lines like: Found valid mod file xxx.jar with {modid} mods
        if let Some(debug) = &self.debug {
            let found: Vec<&str> = debug.lines().filter_map(|l| l.find("valid mod file ").map(|p| &l[p + "valid mod file ".len()..])).collect();
            for kw in &keywords {
                for line in &found {
                    if line.contains(&format!("{{{kw}}}"))
                        && let Some((name, _)) = line.rsplit_once(" with")
                    {
                        names.push(name.to_string());
                    }
                }
            }
        }

        // Fallback when neither source matched: use the file names in the mods folder.
        if names.is_empty() {
            for kw in &keywords {
                let kw = kw.to_lowercase().replace('_', "");
                if kw.len() < 3 {
                    continue;
                }
                names.extend(self.mod_files.iter().filter(|f| f.to_lowercase().replace('_', "").contains(&kw)).cloned());
            }
        }

        dedup(&mut names);
        (!names.is_empty()).then_some(names)
    }

    fn key_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(crash) = &self.crash {
            let body = before_first(crash, "A detailed walkthrough of the error");
            if let Some(desc) = body.lines().find(|l| l.starts_with("Description:")) {
                out.push(desc.to_string());
            }
            let after = body.split_once("Description:").map_or(body, |(_, rest)| rest);
            out.extend(compress_stack(after.lines().skip(1).collect()));
        }
        if out.len() <= 1
            && let Some(hs) = &self.hs
        {
            let lines: Vec<&str> = hs.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                if l.starts_with("#  EXCEPTION") || l.starts_with("# JRE version") || l.contains("Out of Memory Error") || l.contains("out of physical RAM") {
                    out.push(l.to_string());
                }
                if l.starts_with("# Problematic frame:")
                    && let Some(next) = lines.get(i + 1)
                {
                    out.push(next.to_string());
                }
            }
        }
        if out.len() <= 1
            && let Some(mc) = &self.mc
        {
            let lines: Vec<&str> = mc.lines().collect();
            let stack = last_stack(&lines);
            if stack.is_empty() {
                let mut tail: Vec<String> = lines.iter().rev().filter(|l| !l.trim().is_empty()).take(15).map(|l| l.to_string()).collect();
                tail.reverse();
                out.extend(tail);
            } else {
                out.extend(stack);
            }
        }
        out.into_iter().map(|l| tidy_line(&l)).filter(|l| !l.is_empty()).collect()
    }
}

/// Takes the first `head` and last `tail` lines,
/// drops blank and duplicate lines, and ends each line with a newline.
fn head_tail(raw: &[&str], head: usize, tail: usize) -> String {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut picked: Vec<&str> = Vec::new();
    if raw.len() <= head + tail {
        for l in raw {
            if seen.insert(l) {
                picked.push(l);
            }
        }
    } else {
        let mut viewed = 0;
        while viewed < raw.len() && picked.len() < head {
            if seen.insert(raw[viewed]) {
                picked.push(raw[viewed]);
            }
            viewed += 1;
        }
        let mut tail_lines: Vec<&str> = Vec::new();
        for l in raw[viewed..].iter().rev() {
            if tail_lines.len() >= tail {
                break;
            }
            if seen.insert(l) {
                tail_lines.push(l);
            }
        }
        picked.extend(tail_lines.into_iter().rev());
    }
    let mut out = String::new();
    for l in picked {
        let l = l.trim_end_matches('\r');
        if !l.is_empty() {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Extracts keywords that may be mod IDs from a stack.
/// Returns an empty list if there are too many, which usually means a bad match.
fn stack_keywords(stack: &str) -> Vec<String> {
    let mut stacks: Vec<String> = Vec::new();
    for line in stack.lines() {
        stacks.extend(package_prefixes(before_first(line, "{")));
    }
    // Mixin method names look like `handler$zza000$modid$method`.
    // The mod ID is the segment before the method name.
    let mixin = Regex::new(r"at [^(]+?\.\w+\$\w+\$([\w$]+?)\$\w+\(").unwrap();
    stacks.extend(mixin.captures_iter(stack).map(|c| c[1].replace('$', ".")));
    dedup(&mut stacks);

    const IGNORE_PREFIX: [&str; 20] = [
        "java",
        "sun",
        "javax",
        "jdk",
        "oolloo",
        "org.lwjgl",
        "com.sun",
        "net.minecraftforge",
        "paulscode.sound",
        "com.mojang",
        "net.minecraft",
        "cpw.mods",
        "com.google",
        "org.apache",
        "org.spongepowered",
        "net.fabricmc",
        "com.mumfrey",
        "com.electronwill.nightconfig",
        "it.unimi.dsi",
        "MojangTricksIntelDriversForPerformance_javaw",
    ];
    const IGNORE_WORD: &[&str] = &[
        "com",
        "org",
        "net",
        "asm",
        "fml",
        "mod",
        "jar",
        "sun",
        "lib",
        "map",
        "gui",
        "dev",
        "nio",
        "api",
        "dsi",
        "top",
        "mcp",
        "core",
        "init",
        "mods",
        "main",
        "file",
        "game",
        "load",
        "read",
        "done",
        "util",
        "tile",
        "item",
        "base",
        "fake",
        "oshi",
        "impl",
        "data",
        "pool",
        "task",
        "forge",
        "setup",
        "block",
        "model",
        "mixin",
        "event",
        "unimi",
        "netty",
        "world",
        "lwjgl",
        "fakes",
        "fabric",
        "gitlab",
        "common",
        "server",
        "config",
        "mixins",
        "compat",
        "loader",
        "launch",
        "script",
        "entity",
        "assist",
        "client",
        "plugin",
        "modapi",
        "mojang",
        "shader",
        "events",
        "github",
        "recipe",
        "render",
        "packet",
        "preinit",
        "preload",
        "machine",
        "reflect",
        "channel",
        "general",
        "handler",
        "content",
        "systems",
        "modules",
        "service",
        "scripts",
        "network",
        "fastutil",
        "optifine",
        "internal",
        "platform",
        "override",
        "fabricmc",
        "neoforge",
        "external",
        "injection",
        "listeners",
        "scheduler",
        "minecraft",
        "universal",
        "multipart",
        "neoforged",
        "microsoft",
        "transformer",
        "transformers",
        "minecraftforge",
        "blockentity",
        "spongepowered",
        "electronwill",
        "concurrent",
    ];
    let mut words: Vec<String> = Vec::new();
    for s in stacks.iter().filter(|s| !IGNORE_PREFIX.iter().any(|p| s.starts_with(p))) {
        for word in s.trim().split('.').take(4) {
            if word.len() <= 2 || word.starts_with("func_") || IGNORE_WORD.contains(&word.to_lowercase().as_str()) {
                continue;
            }
            words.push(word.trim().to_string());
        }
    }
    dedup(&mut words);
    if words.len() > 10 { Vec::new() } else { words }
}

/// Package names in a line: dot-separated identifiers with the last two segments removed.
/// Those segments are the class and method names.
fn package_prefixes(line: &str) -> Vec<String> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$';
    let mut out = Vec::new();
    let mut rest = line;
    let mut at_line_start = true;
    while let Some(start) = rest.find(is_word) {
        let run_len = rest[start..].find(|c: char| !is_word(c)).unwrap_or(rest.len() - start);
        let mut run = &rest[start..start + run_len];
        // The original regex needs at least one character before the match.
        if at_line_start && start == 0 {
            run = &run[1..];
        }
        at_line_start = false;
        if let Some(p) = longest_package(run) {
            out.push(p);
        }
        rest = &rest[start + run_len..];
    }
    out
}

fn longest_package(run: &str) -> Option<String> {
    let segs: Vec<&str> = run.split('.').collect();
    if segs.len() < 4 {
        return None;
    }
    let ident_start = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    for k in (2..=segs.len() - 2).rev() {
        let prefix = &segs[..k];
        if segs[k].is_empty() || prefix.iter().any(|s| s.is_empty() || s.contains('$')) {
            continue;
        }
        if prefix[0].len() >= 2 && ident_start(prefix[0]) && ident_start(prefix[1]) && prefix[1..].join(".").len() >= 2 {
            return Some(prefix.join("."));
        }
    }
    None
}

/// Tab-indented lines after the marker line, with surrounding whitespace trimmed.
fn indented_after(text: &str, marker: &str) -> Vec<String> {
    let Some((_, rest)) = text.split_once(marker) else { return Vec::new() };
    rest.split('\n').skip(1).take_while(|l| l.starts_with('\t')).map(|l| l.trim().to_string()).collect()
}

/// Solutions printed by Fabric: lines after the marker that start with whitespace
/// and look like `- xxx`.
fn solution_lines(text: &str, marker: &str) -> Vec<String> {
    let Some((_, rest)) = text.split_once(marker) else { return Vec::new() };
    rest.split('\n').skip(1).take_while(|l| l.starts_with(char::is_whitespace) && l.trim_start().starts_with("- ")).map(|l| l.trim().to_string()).collect()
}

/// The last stack trace in the log: the exception line and the indented lines after it.
fn last_stack(lines: &[&str]) -> Vec<String> {
    let is_stack = |l: &str| l.starts_with(char::is_whitespace) || l.starts_with("Caused by:");
    let Some(end) = lines.iter().rposition(|l| l.trim_start().starts_with("at ") && l.starts_with(char::is_whitespace)) else { return Vec::new() };
    let mut start = end;
    while start > 0 && is_stack(lines[start - 1]) {
        start -= 1;
    }
    let start = start.saturating_sub(1);
    compress_stack(lines[start..=end].to_vec())
}

/// Keeps the exception line and up to 6 `at` lines. Each `Caused by` adds up to 2 more.
/// At most 24 lines in total.
fn compress_stack(lines: Vec<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut budget = 0usize;
    for l in lines.into_iter().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        let trimmed = l.trim_start();
        if trimmed.starts_with("at ") {
            if budget == 0 {
                continue;
            }
            budget -= 1;
        } else if trimmed.starts_with("...") {
            continue;
        } else {
            budget = if out.is_empty() { 6 } else { 2 };
        }
        out.push(l.to_string());
        if out.len() >= 24 {
            break;
        }
    }
    out
}

/// Removes the jar and Mixin info at the end of a stack line.
/// Long lines are truncated.
fn tidy_line(line: &str) -> String {
    let mut l = line.trim_end();
    for cut in [" ~[", " {re:", " [?:"] {
        if let Some(p) = l.find(cut) {
            l = &l[..p];
        }
    }
    let l = l.replace('\t', "    ");
    if l.chars().count() > 240 { format!("{}…", l.chars().take(240).collect::<String>()) } else { l }
}

fn seek(text: &str, pattern: &str) -> Option<String> {
    let re = Regex::new(pattern).unwrap();
    re.captures(text).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

fn seek_all(text: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern).unwrap().find(text).map(|m| m.as_str().to_string())
}

fn search(text: &str, pattern: &str) -> Vec<String> {
    let re = Regex::new(pattern).unwrap();
    re.captures_iter(text).filter_map(|c| c.get(1).map(|m| m.as_str().to_string())).collect()
}

fn search_all(text: &str, pattern: &str) -> Vec<String> {
    Regex::new(pattern).unwrap().find_iter(text).map(|m| m.as_str().trim().to_string()).collect()
}

/// Text between the first `a` and the first `b` after it.
/// Empty if `a` is missing, and runs to the end if `b` is missing.
fn between<'t>(text: &'t str, a: &str, b: &str) -> &'t str {
    let Some(p) = text.find(a) else { return "" };
    let rest = &text[p + a.len()..];
    rest.find(b).map_or(rest, |q| &rest[..q])
}

fn before_first<'t>(text: &'t str, sep: &str) -> &'t str {
    text.find(sep).map_or(text, |p| &text[..p])
}

fn dedup<T: PartialEq + Clone>(v: &mut Vec<T>) {
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    *v = out;
}
