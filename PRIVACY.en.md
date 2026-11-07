# CairnLauncher Privacy Policy

Document version: 1.0

Effective date: 2026-11-06

Applies to: CairnLauncher and its command-line tool `cairn`. This policy was written from the source code of software version 0.1.0.

中文版：[PRIVACY.md](PRIVACY.md)

## Key points

- CairnLauncher has no telemetry, usage statistics, crash-report upload, advertising or behavioural tracking, and it does not upload your data to any server operated by the developer.
- Your accounts, settings and game instances are stored on your own device, by default in the `.cairn` folder next to the executable.
- When you sign in, download or search for content, the launcher connects directly to third-party services such as Microsoft, Mojang, Modrinth, CurseForge and any mirrors you enable. These requests are made to carry out your actions; the data is handled by those third parties and does not pass through the developer.
- On Windows, `accounts.dat`, which holds your tokens, is encrypted with DPAPI. On other systems it is encrypted in software with a key derived from local machine information, which is not an operating-system credential store. `config.json` and other files are plain text. See Section 2.
- The developer does not hold your data and therefore cannot look it up, export it or delete it for you. You can delete the local data yourself; see Section 5.

## 1. Introduction

### 1.1 What this policy covers

CairnLauncher (the "Software") is a third-party Minecraft launcher developed by RustCraftMC Team (the "Developer"). This policy explains what data the Software stores on your device, which network services it connects to, and how you can manage that data.

This policy and the Terms of Service ([TERMS.en.md](TERMS.en.md)) are two separate documents. Neither is related to the Software's open-source licence.

### 1.2 Privacy design principles

- No user-data reporting system and no telemetry.
- No behavioural tracking and no advertising tracking, and no third-party statistics, analytics or crash-reporting components. The review of the source code and its dependencies found no such components.
- No upload of user data to the Developer's servers. The source code of Software version 0.1.0 contains no code that sends data to a server operated by the Developer.
- User settings, game instances, account information and similar data are, as a rule, kept on the user's own device and managed by it.
- The Developer does not make money by collecting users' personal information.

### 1.3 Network requests are not data reporting

The Software needs a network connection to sign in, download the game, search for mods and so on. These requests are the direct result of your actions (or of features you enable). They go from your device straight to third-party services and do not pass through any server of the Developer. They are not data reporting by the Developer. Sections 3 and 4 explain, item by item, where requests go and what they contain.

### 1.4 What this policy does not cover

- The Minecraft game itself. The game is provided by Mojang Studios and Microsoft; the game process's own network activity and data handling are governed by their policies.
- Mods, resource packs and plugins that you install, and any network activity they start on their own.
- How third-party services handle data after they receive it.
- Modified or derived versions of the Software.

### 1.5 Open-source licence

The open-source licence of the Software's source code is a separate legal document; see the LICENSE file in the project repository ([RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)). This policy does not repeat its terms.

## 2. Local data storage

### 2.1 Where data is stored

The Software's data directory (written below as `<data directory>`) is by default the `.cairn` folder next to the executable. Setting the environment variable `CAIRN_HOME` moves it elsewhere. This is a portable layout: data sits beside the executable and is not written to the user profile on the system drive.

The rule is the same on Windows, Linux and macOS. The code does not use standard system locations such as `%APPDATA%`, `%LOCALAPPDATA%`, `~/.config` or `~/Library/Application Support` for the launcher's own data. The platform the project declares support for is Windows 10 and later. The code also contains a Linux implementation; there is no macOS-specific implementation and macOS is not declared as supported.

Game folders (`.minecraft` and similar) are separate from the launcher's data directory; see the table below.

### 2.2 Data the Software may store

| Data | Contents | Location |
| --- | --- | --- |
| Minecraft account information | Account type, nickname, UUID, skin URL, token expiry time, time added; for external-authentication accounts, also the authentication server address | `<data directory>/accounts.dat` (the whole file is encrypted) |
| OAuth and sign-in tokens | Microsoft refresh token, Minecraft access token, the application (client) ID used for sign-in; for external-authentication accounts, the access token and clientToken | `<data directory>/accounts.dat` (same as above) |
| Offline accounts | The player name you entered and the offline UUID generated from it. There is no token | `<data directory>/accounts.dat` |
| Launcher settings | Download-source and mirror settings, Java paths, memory and JVM/game arguments, window size, offline-skin settings, registered game folders, the selected instance, whether the agreement has been accepted, whether the first-run wizard is finished; and, if you entered them, a CurseForge API key or a custom Microsoft application ID | `<data directory>/config.json` (plain-text JSON) |
| Appearance and layout | Theme, language, accent colour, the path of the wallpaper image (the image itself is not copied), home widget layout, custom widgets and panels, extension enable state | `<data directory>/appearance.json`, `widgets.json`, `extensions.json`, `widgets/`, `panels/` |
| Play-time statistics | Accumulated play time in seconds by date and instance, used for the heatmap on the home page. Stored on this device only | `<data directory>/stats/playtime.json` |
| Instance settings | Each version's own Java, memory, launch arguments, bound account, icon and so on | Standard layout: `versions/<version>/cairn.json` inside the game folder; standalone instances: `<data directory>/instances/<name>/instance.json` |
| Game files | Versions, libraries, assets, mods, resource packs, shader packs, saves, screenshots, the server list `servers.dat`, and the logs and crash reports the game itself produces | The game folders you have registered. There are two by default: `.minecraft` next to the executable and, only if it exists, `%APPDATA%\.minecraft`. Standalone instances use `<data directory>/shared/` and `<data directory>/instances/` |
| Java runtimes | Official Mojang Java runtimes downloaded by the launcher; the Java detection cache | `<data directory>/runtime/`, `<data directory>/cache/` |
| Download and resource caches | The version manifest (treated as valid for 10 minutes), hash caches, skin and avatar images, icons, the MC Wiki lookup cache (hits kept for 30 days, misses for 3 days), the bundled Java agent and authlib-injector files | `<data directory>/cache/`, which contains `skins/`, `icons/`, `agent/` and `mcmod.json` |
| Temporary files | Temporary files of installers and the game's native libraries | `<data directory>/temp/`, `<data directory>/natives/` |
| Launcher log | The launcher event log, including the data directory path, Java paths, instance names, versions and error messages; when the game crashes, up to the last 500 lines of the game's console output before the crash are also written | `<data directory>/logs/latest.log`; the previous run's log is `latest.log.old` |
| Crash logs and debugging information | `crash-reports/`, `hs_err_pid*.log`, `logs/latest.log` and `debug.log` produced by the game and Java themselves. The launcher only reads and analyses them on this device | The game folder |
| Exported error reports | A zip created only when you click "Export", containing the analysis, system information, launch command, launcher log, game logs, and mod file names and sizes. On export, tokens are masked, as is the user name in paths shaped like `Users\username` | Chosen by you in the save dialog; the suggested default is the desktop |
| Plugin data | Plugin packages you installed, plugin enable and permission state, and the plugins' own storage | `<data directory>/plugins/`, `plugins.json`, `plugin-data/<plugin ID>/` |

Information that exists only in memory and is never written to disk: the result of the region check (see Section 3 of the Terms of Service), and the external-authentication password you type into the interface (the input box is cleared after you submit it).

Information the Software reads on this device but does not store or upload: the system time zone and region settings, the operating-system version, physical memory, system proxy settings, installed Java runtimes, the system light/dark theme and display scaling; and, when you use the "Import" feature, the configuration and game folders of other launchers such as PCL2 and HMCL, so that versions and settings can be imported.

### 2.3 How sensitive information is stored

- **Windows.** The whole of `accounts.dat` is encrypted with Windows DPAPI (`CryptProtectData`) as the current Windows user. This means that a different Windows user, or a copy of the file on another computer, cannot read the contents directly. The limitation is that any other program running as your Windows user can also call DPAPI to decrypt it: DPAPI does not isolate applications from each other, and the launcher sets no extra password or entropy.
- **Linux and other non-Windows systems.** The whole file is encrypted with AES-256-GCM. The key is derived with SHA-256 from `/etc/machine-id` (or `/var/lib/dbus/machine-id`), the environment variables `USER` and `HOME`, and a fixed string written in the source code. On systems without a machine-ID file (for example macOS) a fixed fallback string in the source code is used instead, which is weaker. The Software does **not** use the macOS Keychain, the Linux Secret Service (libsecret, GNOME Keyring, KWallet) or any other operating-system credential store. A person or program that can read the machine ID and the environment variables can recompute the key, so this encryption only prevents the file from being read directly after it is copied away; it is weaker than a system credential store.
- **Other files are not encrypted.** `config.json`, `appearance.json`, logs and caches are plain text. A CurseForge API key or custom Microsoft application ID that you enter is stored in plain text in `config.json`.
- **While the game runs.** The launcher passes the access token to the game process as the `--accessToken` launch argument, which is the standard way Minecraft works. Other programs on the same system may be able to see a process's launch arguments. Offline accounts do not use a real token.

### 2.4 What the Software does not store

The Software does not store your Microsoft account password: you type your account name and password into Microsoft's own sign-in page, which the launcher cannot see. The password for external authentication (Yggdrasil) is used only for a single sign-in request and is not written to disk.

When you sign in, the launcher automatically copies the one-time device code that Microsoft issues to the system clipboard, so that you can paste it into the sign-in page. The code is valid only for a short time and only for this one sign-in; other programs on the same system may be able to read the clipboard.

## 3. Third-party services

### 3.1 Services the Software may connect to directly

The "When it connects" column distinguishes operations you trigger from requests that happen automatically.

| Service | Purpose and when it connects | What is sent |
| --- | --- | --- |
| Microsoft identity platform (`login.microsoftonline.com`) | Signing in to a Microsoft account (OAuth device-code flow). Connects when you click sign-in; afterwards tokens are refreshed automatically when they expire | The application (client) ID, the requested scopes (`XboxLive.signin`, `offline_access`) and the device code; on refresh, the refresh token. The account name and password you type on Microsoft's page (for example `aka.ms/devicelogin`, opened in your system browser) go only to Microsoft |
| Xbox services (`user.auth.xboxlive.com`, `xsts.auth.xboxlive.com`) | Exchanges the Microsoft token for Xbox tokens. Connects at sign-in and refresh | The Microsoft access token and the Xbox token |
| Minecraft services (`api.minecraftservices.com`) | Obtains the Minecraft access token and reads the game profile (UUID, player name, skins). Connects at sign-in and refresh | The XSTS token; afterwards the Minecraft access token is used to read the profile |
| Mojang player and skin services (`api.mojang.com`, `sessionserver.mojang.com`, `textures.minecraft.net`) | Happens automatically when account avatars are loaded (for example at start-up). For a Microsoft account the skin image URL comes from the game profile, and the avatar is downloaded from that URL (normally on `textures.minecraft.net`) and cached; for an offline account, when the skin source is Mojang (the default), the skin is looked up by player name. When the offline skin feature is on (the default), the launcher-agent inside the game also looks skins up by player name in the same way | The player name (offline accounts); your IP address |
| Mojang and Minecraft download services (`piston-meta.mojang.com`, `launchermeta.mojang.com`, `launcher.mojang.com`, `piston-data.mojang.com`, `libraries.minecraft.net`, `resources.download.minecraft.net`) | Version manifest, the game itself, libraries, assets and official Java runtimes. Connects when the launcher starts (if the local version manifest is older than 10 minutes), when you install a version, when files are completed before launch, and when Java is downloaded | The requested file path (that is, the version you chose); your IP address; the User-Agent `CairnLauncher/<version>` |
| Download mirrors: BMCLAPI (`bmclapi2.bangbang93.com`, `openbmclapi.bangbang93.com`), MCIM (`mod.mcimirror.top`), and any mirrors you add yourself | BMCLAPI and MCIM are enabled by default and the download strategy is "Auto": when downloading, the launcher sends one speed-test request to the official source and to mirrors such as BMCLAPI (the first 64 KiB of the version manifest, at most once every 30 minutes; MCIM does not take part in the speed test) and picks a source by speed; before the test, mirrors are tried first | The same request content as for official sources. Mirrors are run by third parties and can see your IP address and what you download. In addition, when no CurseForge API key is configured, CurseForge search and metadata requests are always sent to MCIM, regardless of the download-source settings |
| Modrinth (`api.modrinth.com`, `cdn.modrinth.com`) | Searching, viewing projects, installing, checking mod updates, exporting modpacks. All triggered by your actions | Search terms and filters (game version, loader, category), project and version IDs. When you install a resource or check for mod updates, the SHA-1 hashes of the mod files in the instance are sent; when you export a `.mrpack`, so are those of resource packs and shader packs. File names and file contents are not sent |
| CurseForge (`api.curseforge.com`; MCIM when there is no key) and its file downloads (`edge.forgecdn.net`, `mediafilez.forgecdn.net`) | Searching and installing. Triggered by your actions | Search terms, filters, mod and file IDs; the API key, if you configured one |
| MC Wiki (`search.mcmod.cn`) | Looks up Chinese names and summaries of mods. Happens only when the interface language is Simplified Chinese and you search online in the "Mods" category, and only when the built-in offline index and the local cache cannot answer; when it connects, it also looks up the titles of up to 6 search results | The search term and the titles of search results, using a browser-style User-Agent. Results are cached locally |
| LittleSkin or the external authentication server you enter (Yggdrasil / authlib-injector; `littleskin.cn` by default) | External sign-in, and skin lookup when you choose LittleSkin as the skin source. While the game runs, authlib-injector makes the game connect to that server directly | The user name or e-mail and password you type, sent only to the server you chose; afterwards the access token is used to refresh. A server you enter is controlled by its operator and the launcher cannot verify that it can be trusted; if you enter an `http://` address, the communication is not encrypted |
| Hitokoto (`v1.hitokoto.cn`) | Fetches one quotation each time the launcher starts, and again when you click "Refresh" in the "Quote" widget. There is currently no separate switch to turn it off | A plain HTTPS request with no account information; the other side can see your IP address |
| Launcher update checking (`rustcraftmc.github.io`, GitHub Releases & mirrors) | Check for and download updates to the launcher. Runs quietly in background on startup by default, or manually triggered in Settings; can be disabled | Currently running launcher version, system architecture (`windows-x86_64`), UI language, IP address; downloads release binary and validates SHA-256 locally |
| Loader and installer sources: Forge (`maven.minecraftforge.net`, `files.minecraftforge.net`), NeoForge (`maven.neoforged.net`), Fabric (`meta.fabricmc.net`, `maven.fabricmc.net`), Quilt (`meta.quiltmc.org`), OptiFine (`optifine.net`, or via BMCLAPI), authlib-injector (via BMCLAPI, or `authlib-injector.yggdrasil.gservice.top`) | Connects when you install the corresponding loader, or the first time you launch the game with an external account | The requested file path and your IP address |
| Your browser | Web pages the launcher opens in your default browser, for example the sign-in page, Minecraft and Microsoft terms, GitHub links and MC Wiki pages. They open only when you click | Handled by the browser; the policy of the website concerned applies |
| Plugin network access | A JavaScript plugin that you install and grant the "network" permission can make HTTP/HTTPS requests to any host. For example, the example plugin `pcl-news` in the repository requests news content from `news.bugjump.net` | Decided by the plugin. Plugins are third-party code; see 6.6 |

### 3.2 Three kinds of data

- **Data actively collected by the Developer:** none.
- **Data sent to third-party services to fulfil your request:** the contents listed in the table above. It is sent for the functions you asked for, such as signing in, downloading and searching.
- **Data processed independently by the third-party service itself:** how a third party records, stores and uses a request after receiving it (for example access logs and account data) is decided by that third party under its own policy. The Developer cannot control it and cannot access or delete it on your behalf.

### 3.3 The terms of these services also apply

When you use these services you must also comply with their own terms of service and privacy policies, for example the [Microsoft Privacy Statement](https://privacy.microsoft.com/privacystatement), the [Microsoft Services Agreement](https://www.microsoft.com/servicesagreement) and the [Minecraft End User License Agreement](https://www.minecraft.net/eula). The Developer cannot control how third-party services handle data and is not responsible for it.

## 4. Network communication

### 4.1 Normal network behaviour

- OAuth and authentication requests to Microsoft, Xbox and Minecraft services when you sign in.
- Download requests when you install, launch the game, or let the launcher complete missing files.
- Requests to Modrinth, CurseForge and similar services when you search for mods, install resources, check for mod updates or export a modpack.
- Requests that happen automatically when the launcher starts: the version manifest (when the local cache has expired), loading an avatar for each account (which may trigger a skin lookup), and fetching one quotation.
- Mirror speed-test requests while downloading.
- Update checks and downloads for the launcher itself: the launcher requests a static manifest (`update.json`) from the project website via HTTP GET to compare version numbers (checked quietly in the background on startup by default, which can be disabled in Settings). When an update is detected, the new binary is downloaded from GitHub Releases only after your explicit confirmation, verified via SHA-256, and swapped in place upon restart. No account credentials or device identification data are sent.
- The Software has no telemetry, usage statistics, crash upload or advertising requests.

### 4.2 What the other side can see

A server that receives a request can normally see your IP address, the requested URL and path, the time, the User-Agent (`CairnLauncher/<version>` for most requests; the MC Wiki request uses a browser-style User-Agent) and TLS handshake information. Account tokens are sent only to Microsoft, Xbox and Minecraft services and to an external authentication server that you choose yourself.

By default the Software connects directly and does not use the system proxy, so these services see your real IP address. You can make it follow the system proxy on the download page of "Settings".

### 4.3 These requests are not data reporting

All of the above requests go to third-party services to carry out functions you asked for. They do not mean that the Software can report user data to the Developer. Network connections made by the game process and by mods themselves are outside the Software's control.

## 5. Data deletion

### 5.1 Removing a Minecraft account

On the "Accounts" page, click the delete button on the right of the account and confirm; the account record and its tokens are then removed from `accounts.dat`. This deletes only the copy on this device; the authorisation already granted to Microsoft is not revoked by it. To revoke it, review and revoke authorised applications in your Microsoft account settings; the exact place is as shown by Microsoft's pages.

### 5.2 Clearing local caches

The Software has no one-click button for clearing caches. After exiting the launcher you can delete `<data directory>/cache/`, `<data directory>/temp/`, `<data directory>/natives/` and the logs in `<data directory>/logs/` by hand. They are regenerated or downloaded again when needed.

### 5.3 Deleting the configuration file

Deleting `<data directory>/config.json` resets settings to their defaults and also clears the record of having accepted the agreement, so the agreement and the first-run wizard are shown again at the next start.

### 5.4 Deleting the data directory

After exiting the launcher, delete the whole `<data directory>` (by default the `.cairn` folder next to the executable). It contains accounts, settings, caches, logs, automatically downloaded Java, standalone instances and the shared resource pool (`instances/`, `shared/`), and plugins and their data.

### 5.5 Uninstalling the launcher is not the same as deleting game data

The Software is portable and has no installer; "uninstalling" means deleting the executable and the data directory. Game instances, mods, saves and resource packs are stored in game folders (by default `.minecraft` next to the executable, `%APPDATA%\.minecraft`, and any folders you added), which may be separate from the launcher's data directory, so uninstalling the launcher does not necessarily delete them; conversely, deleting `<data directory>` does not delete those game folders. Please note in particular that the default `.minecraft` sits right next to the executable: if you delete the whole folder, your saves are deleted too, so back them up first.

### 5.6 Data held by third parties

Data held by third parties such as Microsoft, Modrinth, CurseForge, mirrors and skin sites must be requested from them. The Developer has no data about you and therefore cannot delete it for you.

## 6. Data security

### 6.1 Protection of account data

How accounts and tokens are protected is described in 2.3. On Windows it relies on DPAPI; on other systems it is not a system credential store and is weaker.

### 6.2 Token risks and recommendations

A refresh token can renew a sign-in without asking for your password again, so if someone else obtains it they may be able to use your signed-in state. We recommend: use a password-protected system account; turn on disk encryption where you can; do not send `<data directory>` or `accounts.dat` to anyone; if you suspect a leak, first remove the account, then revoke the authorisation in your Microsoft account settings and change your password.

### 6.3 Access permissions of local files

The Software does not set file access permissions of its own and uses the operating system's defaults for the folder it is in (on Unix-like systems only, it marks downloaded Java executables as executable). If you put the launcher where several people can read (a shared folder, a public computer, a removable drive), others may be able to read the plain-text `config.json`, the logs, the caches and the encrypted `accounts.dat`.

### 6.4 Sensitive information that may appear in logs

The launcher log `latest.log` is written without redaction. What the launcher writes itself is mainly paths, Java paths, instance names, versions and error messages; paths may contain your system user name. When the game crashes, buffered game output is written to the log. `hs_err_pid*.log`, which Java writes to the game folder after a native crash, may contain the full launch command line, which may include the access token; the launcher does not rewrite these original files.

Exported error reports mask tokens and user names in paths, but automatic masking is not guaranteed to be complete. Please check a report yourself before sharing it.

### 6.5 Transport security

The Software uses HTTPS by default when it connects to the services above. Unencrypted transport may be used in two cases: when you manually enter an address starting with `http://` (an external authentication server, a custom mirror and so on), in which case account passwords or tokens may be intercepted; and when the skin image address recorded in a Minecraft profile starts with `http://`, in which case the avatar image is downloaded from that address as given, which is only a public skin image. The launcher checks that the URL of the Microsoft device-sign-in page belongs to a Microsoft domain and otherwise falls back to `https://aka.ms/devicelogin`.

### 6.6 Plugins and mods

Plugins are third-party code and run in a separate process. A plugin must declare permissions for capabilities such as network access, reading and writing arbitrary files, running external programs or changing launch arguments, and you confirm them when you enable it. Reading and writing arbitrary files, running external programs and changing launch settings are high-risk permissions. Mods run inside the game process and the Developer cannot restrict their behaviour. Install only plugins and mods that you trust.

### 6.7 No promise of absolute security

No software can guarantee absolute security. The Developer has taken the measures described above but cannot guarantee that they work in every situation.

## 7. Children's privacy

### 7.1 How the Software actually works

The Software processes data locally on your device, and the Developer neither receives nor stores users' personal information. The Software does not ask you to register an account with the Developer, does not collect age or date of birth, sets no age threshold, and has no built-in parental controls or play-time limits.

### 7.2 Third-party accounts and services

Sign-in uses a Microsoft account, and Microsoft has its own rules for minors' accounts. For example, a minor's account must join a family group before it can sign in, and in that case the launcher only shows the message returned by Microsoft. Third-party services such as Modrinth and CurseForge also have their own age and parental-consent requirements.

### 7.3 Laws in different regions

Different regions regulate children's personal information differently, for example Article 31 of the PRC Personal Information Protection Law and Article 72 of the PRC Minors Protection Law (for minors under 14, handling their personal information requires the guardian's consent), Article 8 of the EU GDPR (where information society services are offered directly to a child and processing is based on consent, Member States set the age threshold between 13 and 16), and the US COPPA (aimed at operators that collect personal information from children under 13). Because the Developer does not collect personal information, the Developer has not set up a separate guardian-consent process. Whether these rules impose any requirement on the Developer in a particular case is discussed in Section 8.

### 7.4 Advice for guardians

Minors should use the Software under the guidance of a guardian. Guardians can manage a child's accounts through Microsoft Family Safety settings and the settings of each third-party service. If you believe a third-party service has collected a child's information, please contact that service directly.

## 8. Regional laws and user rights

### 8.1 How the applicable law is assessed

This section is based on how the Software actually processes data. The Software's data is processed locally on the user's device, and the Developer neither collects nor receives users' personal information, so the Developer normally does not store or process users' personal information. This section lists the main laws of each region and your rights. It reaches no conclusion on whether those laws apply to the Developer; if the law where you live applies, your rights are to be exercised against the party that actually handles your personal information.

### 8.2 Mainland China

- Personal Information Protection Law: a personal information handler is an organisation or individual that independently decides the purposes and means of handling in personal information handling activities (Article 73). The law does not apply to a natural person handling personal information for personal or family affairs (Article 72). Individuals have the rights to know, to decide, to access, to copy, to correct, to supplement, to delete and others (Articles 44 to 50), exercised against the handler. The Developer does not handle your personal information, so there is no data held by the Developer against which you could exercise these rights; you can exercise them against the third-party services. When you use the Software, your device sends requests directly to servers outside China (such as Microsoft and Mojang); the Developer does not receive that data and does not provide it abroad on your behalf.
- Cybersecurity Law (as amended in 2025, in force from 1 January 2026) and Data Security Law: the Software has no server side operated by the Developer, and the Developer does not collect or store user data.
- Minors Protection Law and Regulations on the Protection of Minors Online: see Section 7.
- Your rights and interests in personal information under the Civil Code (including Article 1034) are not affected.

### 8.3 European Union and European Economic Area

- The GDPR calls the party that determines the purposes and means of processing personal data the controller (Article 4(7)). The Developer holds no server-side user data and does not decide how data on the device is processed, so it is normally not the controller of that data. The GDPR does not apply to a user who processes data in the course of a purely personal or household activity (Article 2(2)(c)). Third-party services such as Microsoft are controllers of the data they process themselves.
- If the GDPR applies, you have rights of access, rectification, erasure, restriction of processing, objection and data portability, and the right to lodge a complaint with a supervisory authority. These rights are exercised against the controller concerned.
- It should be noted that in some cases the Software sends your IP address to third parties automatically, without a direct action by you, for example fetching a quotation at start-up and mirrors enabled by default (see Section 3). These requests are handled by the third parties concerned under their own policies.

### 8.4 United States

- Section 5 of the Federal Trade Commission Act prohibits unfair or deceptive practices. This policy states only behaviour that can be confirmed from the source code.
- COPPA: the Developer collects no personal information at all, including children's personal information.
- State privacy laws (the California CCPA/CPRA, and the laws of Virginia, Colorado, Connecticut, Texas and other states) generally set their scope by business size, the number of residents whose data is processed, or the sale of personal data. The Developer does not collect personal information, does not sell it, and does not share it for cross-context behavioural advertising. This policy does not assume that these laws all apply to the Developer.
- If the applicable law gives you rights to know, delete, correct or opt out of sale, there is nothing for the Developer to process because it does not hold your data; for data held by third parties, exercise your rights against the third party.

### 8.5 Non-waivable rights

This policy does not exclude any non-waivable rights you have under the laws applicable where you live.

## 9. Policy updates

This policy may be updated because of changes in the Software's features, changes in legal requirements, or changes in how data is processed. Each version states its document version and effective date (see the top of this document). The latest and earlier versions are available in the project repository ([RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)), and the current version can also be viewed in the launcher under "Settings → About".

The Developer will not invent update dates: the version and date at the top are those actually published. Changes with a significant effect on your rights will be notified as required by applicable law, and where your consent must be obtained again, that is also handled in accordance with applicable law.

## 10. Contact

If you have questions about this policy, please contact the Developer through the Issues page of the project repository. Please do not post tokens, account files or unchecked logs on public pages.

Project website: [rustcraftmc.github.io/cairnlauncher](https://rustcraftmc.github.io/cairnlauncher)

Project repository: [RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)
