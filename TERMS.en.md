# CairnLauncher Terms of Service

Document version: 1.0

Effective date: 2026-11-06

Applies to: CairnLauncher and its command-line tool `cairn`. These terms were written from the source code of software version 0.1.0.

中文版：[TERMS.md](TERMS.md)

## Important notice

Please read these terms in full before you install, start or use CairnLauncher. The following terms have a significant bearing on your rights and obligations and are highlighted here, with the chapter in which each appears shown in brackets:

1. **Account and game-licence rules (Chapter 3).** A user who lives outside mainland China must first sign in, through Microsoft's official flow, to a genuine account that owns Minecraft: Java Edition before adding an offline account. A user who lives in mainland China is strongly advised to use the Software with a genuine licence. An offline account is not a game licence.
2. **The Software is provided as it is (Chapter 10).** You must guard against risks such as game crashes, compatibility problems and data loss yourself, including by backing up your instances, saves and configuration.
3. **Limitation of liability (Chapter 11).** To the extent the law allows, the Developer's liability for indirect loss and similar damage is limited. Liability that the law does not allow to be excluded is not affected, including loss caused by intent or gross negligence, and personal injury.
4. **Termination (Chapter 12).** If you seriously breach these terms, the Developer may take reasonable measures to the extent the law allows.
5. **Changes to these terms (Chapter 16).** The terms may be updated.
6. **Your non-waivable statutory rights are not affected by these terms (Chapter 18).**

The meaning and consequences of these terms are explained further in the chapters concerned. If you have questions, please contact the Developer through the project repository (see Chapter 17) before you use the Software.

## 1. Introduction

### 1.1 What these terms are

These terms are an agreement between you (the "User") and RustCraftMC Team (the "Developer"), the developer of CairnLauncher (the "Software"), about your use of the Software.

### 1.2 Read before you use

Before you install, start or use the Software, you should read and understand these terms.

### 1.3 How you accept

When you first start the launcher it shows these terms and the Privacy Policy. You can tick "I have read and agree" and click "Agree and continue" only after you have scrolled to the end of the full text. Where there is no such screen (for example the command-line tool `cairn`, or a version you compiled yourself), you accept these terms by installing, running or using the Software. If you do not agree, do not use the Software.

### 1.4 Scope

These terms apply to the Software itself and the features it provides, including the graphical interface and the command-line tool. They do not apply to the Minecraft game itself, to third-party services, or to mods, resource packs and plugins that you install; those are governed by their own terms (see Chapters 6 and 7).

### 1.5 Relationship to the Privacy Policy

The Privacy Policy ([PRIVACY.en.md](PRIVACY.en.md)) is a separate document. These terms do not contain details about the handling of personal information; the Privacy Policy governs that.

### 1.6 Version, date and language

The version number and effective date of these terms are at the top. The terms exist in Simplified Chinese and English, and their content is the same.

## 2. Description of services

### 2.1 What the Software provides

CairnLauncher is a third-party tool for managing Minecraft. It currently provides these functions:

- Minecraft account management: Microsoft accounts, offline accounts and external-authentication accounts, any number of them, with the option of binding an account to an instance.
- Microsoft OAuth authentication: signing in through Microsoft's official device-code flow.
- Game version management: installing officially released versions, and loaders such as Fabric, Quilt, Forge, NeoForge and OptiFine.
- Game instance management: standard layout, version isolation and standalone instances; importing `.mrpack`, CurseForge and MultiMC/Prism modpacks and exporting `.mrpack`; importing versions and settings from the folders of launchers such as PCL2 and HMCL.
- Java environment management: detecting installed Java and, when needed, downloading the Java runtimes that Mojang provides.
- Launching and running the game: starting the game according to your settings, analysing logs on this device when the game exits abnormally, and exporting error reports.
- Mod and resource pack management: enabling or disabling them locally, searching and installing from Modrinth and CurseForge, and checking mod updates through Modrinth.
- Third-party resource downloads: downloading game files and resources from Mojang's official servers and from mirrors you enable.
- Plugins and extensions: JavaScript plugins and YAML widgets and panels.

### 2.2 Launcher Update Mechanism

The Software supports checking for and downloading updates:
- By default, it quietly checks in the background upon startup to see if a newer version has been released. You can also manually check for updates in "Settings - About" or disable automatic checking at any time.
- When an update is available, you decide whether to download it. Once downloaded and verified via SHA-256 checksums, replacement occurs only after you explicitly click to restart.
- The Software never forcibly overwrites itself without user confirmation; all updates remain under user control.

### 2.3 What the Software is not

The Software is only a third-party tool for managing the game. It does not sell Minecraft game licences, does not provide Minecraft accounts, does not operate game servers, and does not represent Mojang or Microsoft. The launcher itself neither bundles nor distributes Minecraft game files. The game files are downloaded, as you direct, from Mojang's official servers or, if you enable mirrors, from third-party mirrors. The Software does not promise functions that do not exist in its code.

## 3. Minecraft account and game licence

This chapter is an important part of these terms; please read it carefully.

### 3.1 Where the game licence comes from

The licence to play Minecraft is provided by Mojang Studios and Microsoft. When you use Minecraft you must comply with the [Minecraft End User License Agreement (EULA)](https://www.minecraft.net/eula), the [Minecraft Usage Guidelines](https://www.minecraft.net/en-us/usage-guidelines) and the [Microsoft Services Agreement](https://www.microsoft.com/servicesagreement). CairnLauncher does not sell, transfer or provide Minecraft game licences, and does not grant any licence on behalf of Mojang or Microsoft.

### 3.2 Your responsibilities

You are responsible for the game accounts you use and for their lawfulness. You must not use the Software to impersonate another person's account or to obtain account authorisation unlawfully. Signing in with a Microsoft account must use Microsoft's official authorisation flow.

### 3.3 The three kinds of account in the Software

- **Microsoft account.** You sign in through Microsoft's official device-code flow: you type your account name and password on Microsoft's own page, and the Software cannot see them. After sign-in, the Software reads the account's game profile from the Minecraft service. If no profile can be read, sign-in fails and the message says that the account may not have purchased Minecraft or may not have created a game profile yet. **The Software uses only "whether the Minecraft service can return a game profile for this account" as its test.** It does not call Microsoft's or Mojang's game purchase entitlement interface, does not distinguish how the game was obtained, and does not keep re-checking after sign-in. A successful sign-in therefore shows only that a Minecraft game profile exists for the account; it is not a certification or guarantee of your game licence.
- **Offline account.** A local game-identity configuration provided by the Software; see 3.7.
- **External-authentication account.** A local account record created by signing in to a third-party authentication server (for example LittleSkin, or a Yggdrasil / authlib-injector server that you enter yourself). It has nothing to do with a Microsoft account or a Minecraft game licence, and the authentication server is run by a third party.

### 3.4 Users who live in mainland China

For users who live in mainland China:

- You are strongly advised to use the Software only if you hold a genuine licence for Minecraft: Java Edition.
- You are advised to sign in with a Microsoft account that holds an official licence for Minecraft: Java Edition.
- The Software supports the ways of using offline accounts that its code actually permits: when the Software determines that you are in mainland China (see 3.6), you can add an offline account without first signing in to a Microsoft account.
- You must not treat an offline account as a genuine Minecraft licence or a game licence (see 3.7).

For users in mainland China, holding a genuine account is a recommendation, not a precondition for using all functions of the Software.

### 3.5 Users who live outside mainland China

For users who live outside mainland China, the following are conditions of using the Software:

1. You must hold a valid genuine Minecraft account and the corresponding licence to play the game;
2. Before you use the offline-account function, you must first sign in to your genuine Minecraft account through Microsoft's official authorisation flow and complete the verification that the Software actually implements, that is: the Software's Microsoft device-code sign-in succeeds and the Minecraft service returns a game profile for the account (see 3.3);
3. Only after that verification may you add a new offline account. In the Software's current implementation, adding a new external-authentication account is subject to the same precondition;
4. You must make sure your genuine game licence stays valid and comply with the relevant terms of Microsoft and Mojang.

This section is the Software's own condition of service. It is not a uniform requirement that Mojang or Microsoft impose on all third-party launchers, and it is not a legal requirement of any country or region.

This condition applies to every way in which you use the Software, including the graphical interface and the command-line tool. Whether a given way of use is technically restricted does not change how this condition binds you.

### 3.6 How the Software decides "mainland China"

The Software uses the system settings on your device to decide whether you are in mainland China (which does not include the Hong Kong Special Administrative Region, the Macao Special Administrative Region or Taiwan). It does not use IP geolocation, and it does not ask where you live.

- **Windows:** the system time zone is China Standard Time, and the system "Country or region" setting is China (CN).
- **Linux and other Unix-like systems:** the time zone (the environment variable `TZ`, or the path that `/etc/localtime` points to) contains one of Shanghai, Chongqing, Urumqi, PRC or China Standard Time, and the locale variable (taking the first one that is set, in the order `LC_ALL`, `LC_MESSAGES`, `LANG`) begins with `zh_CN`.

The result is used only while the Software is running, to decide whether to open functions such as adding an offline account. It is not saved or uploaded.

This check relies on system settings, not on where you live or your legal status, and it may not match reality. For example: a person who lives in mainland China but has different system settings is treated as being outside mainland China; a person who lives abroad but has system settings for mainland China is treated as being in mainland China; and system settings can be changed.

The rules in this chapter apply according to where you actually live, not according to the Software's decision. You must not change your time zone, region or language settings in order to get around 3.5. If the Software restricts a function because your settings do not match, sign in with a Microsoft account first.

### 3.7 Definition and limits of an offline account

An offline account is a local game-identity configuration provided by the Software: you enter a player name, and the Software generates an offline identifier (UUID) from it on this device.

- An offline account is not a Microsoft account.
- An offline account does not provide a Minecraft game licence.
- An offline account does not automatically get the skins, game ownership or online-authentication benefits of a genuine account. The Software can, according to your settings, show a public skin for an offline account; this does not mean you own that skin, nor that you obtain any benefit of the corresponding genuine account.
- An offline account may not be able to join Minecraft multiplayer servers that require genuine identity verification.
- You must still comply with the rules of the game server operator.
- The Software does not guarantee that an offline account can access all multiplayer services.

An offline account does not constitute a Minecraft game licence and must not be understood as one.

### 3.8 How signed-in accounts and offline accounts relate

- An offline account is an independent local record and is not tied to a Microsoft account.
- Where 3.5 applies, adding an offline account requires that the Software's account list already contains an account record of the Microsoft type. That is all the Software checks.
- If you later remove the Microsoft account, offline accounts you have already added are not deleted automatically, but you cannot add new offline accounts until you sign in again. When you start the game with an offline account, the Software does not re-check your game licence.
- These checks are only technical measures. Your obligation to keep your game licence valid (3.5, item 4) is not limited by whether the Software checks it.

## 4. Accounts and security

- You must lawfully use only Microsoft accounts that you own or are authorised to use.
- You must keep your account information safe, including the account file `accounts.dat` and access tokens, and not share it with others.
- You must not impersonate another account by reverse engineering, forging authentication information, tampering with the authentication flow or similar means.
- You must comply with Microsoft's rules on account security.
- Bans, recovery and authorisation disputes concerning a Microsoft account itself are handled by Microsoft, not by the Developer.
- An account record in the Software is local configuration, not an official Microsoft account. Deleting a record in the Software does not delete or revoke your Microsoft account or its authorisation.
- How the Software stores tokens is described in Section 2 of the Privacy Policy. If you find a security vulnerability in the Software, please contact the Developer through the project repository, and avoid disclosing exploitable details on public pages.

## 5. Software licence

### 5.1 Open-source licence

The source code of the Software is planned to be licensed under the GNU Lesser General Public License (LGPL); the exact version is as stated in the LICENSE file in the root of the project repository.

Your rights to use, modify and distribute the Software are governed by that licence. These terms do not impose any additional restriction on the rights the LGPL already grants you; if these terms conflict with the LGPL over your rights to use, modify and distribute the source code, the LGPL prevails.

### 5.2 What is outside the licence

The intellectual property in Minecraft, Mojang, Microsoft and third-party resources (mods, resource packs, textures, modpacks and so on) is not within the scope of the Software's open-source licence.

### 5.3 Third-party components

The Software uses other third parties' open-source components and assets, which follow their own licences; for example, the interface library `cairn-ui` is licensed under MIT or Apache-2.0. The list of components and their licences is in the project repository ([RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)).

## 6. Intellectual property

- The licence for the Software's own code is the LGPL described in Chapter 5.
- The rights in CairnLauncher's name, logo and original interface assets belong to the respective rights holders.
- The names, trademarks, game assets and related intellectual property of Minecraft, Mojang and Microsoft belong to their respective rights holders. Some of the Software's version icons are taken from PCL2's resources, and the copyright in them belongs to their respective authors.
- Third-party mods, resource packs, textures and modpacks follow the licences of their original authors.
- You must not redistribute Minecraft game files without authorisation.
- When you use the name of Minecraft, its assets or related content, you must follow the official Minecraft Usage Guidelines.

Official non-affiliation statement:

> NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
>
> 非 Minecraft 官方产品，未获 Mojang 或 Microsoft 批准，也与 Mojang 或 Microsoft 无关联。

## 7. Third-party services

### 7.1 Third-party services the Software connects to

| Service | Use in the Software |
| --- | --- |
| Microsoft identity platform | OAuth sign-in to Microsoft accounts and token refresh |
| Xbox services | Exchanging the Microsoft token for Xbox tokens |
| Minecraft services and Mojang services | Obtaining the Minecraft access token, reading the game profile, skin lookups, and downloading the game itself, libraries, assets and official Java runtimes |
| Modrinth | Searching for, downloading and update-checking mods, resource packs, shader packs and modpacks |
| CurseForge | The same; accessed through the MCIM mirror when there is no API key |
| Game and Java download sources and mirrors | Mojang's official servers, BMCLAPI, MCIM, and mirrors you add yourself |
| Loader and installer sources | Downloads of Forge, NeoForge, Fabric, Quilt, OptiFine and authlib-injector |
| External authentication and skin services | LittleSkin, or a Yggdrasil / authlib-injector server you enter |
| MC Wiki and Hitokoto | Chinese mod names and summaries; the quotation on the home page |
| Plugins you install | Decided by the plugin; they may access any host |

### 7.2 Independent terms

Third-party services have their own independent terms of service and privacy policies, and you must comply with them when you use those services. CairnLauncher does not represent these third-party platforms. The Developer does not guarantee that third-party services remain available and is not responsible for the conduct of third-party platforms themselves. How third-party services handle data is described in Section 3 of the Privacy Policy.

## 8. Acceptable use

You must not:

1. use the Software to carry out unlawful activity;
2. use the Software to infringe others' intellectual property;
3. redistribute Minecraft game files without authorisation;
4. forge or assume another person's account identity;
5. maliciously attack or damage the Software or third-party services, including abusing mirrors and interfaces;
6. exploit a vulnerability, or alter system settings, to get around the account-use restrictions that the Software expressly sets (see Chapter 3);
7. engage in other conduct that violates applicable law or the rules of third-party services.

## 9. Updates and changes to the service

- The launcher supports update checking and user-confirmed in-place updates (see Section 2.2), which can be disabled in Settings at any time.
- The Developer may release new versions to fix defects, adjust functions or improve security. Older versions may no longer be supported.
- When a third party's interface, download source or authentication service changes, some functions may be affected, for example a change in the Microsoft sign-in flow, an interface key becoming invalid, or a mirror ceasing service.
- The Developer may stop or adjust services and functions as circumstances require, while complying with the notice and other obligations in applicable law.

## 10. Disclaimer

To the extent permitted by applicable law:

- The Software may contain defects and compatibility problems.
- A game crash may be caused by many factors, such as the game version, Java, graphics drivers, mods and hardware, and is not necessarily caused by the Software. The Software's crash analysis is only an aid and is not guaranteed to be accurate.
- Interruptions or changes of third-party services (Microsoft, Mojang, mirrors, mod platforms and so on) are not entirely within the Developer's control.
- Modifying game files yourself, or installing third-party mods or plugins, may cause abnormal behaviour.
- Game instances, saves and configuration files should be properly backed up by you.

This chapter is not an unlimited disclaimer. It does not exclude the non-waivable rights that applicable law gives to consumers.

## 11. Limitation of liability

To the extent permitted by applicable law, the Developer is not liable for indirect loss, loss of profit or loss of data that you suffer from using or being unable to use the Software.

The following are not covered by the preceding paragraph and are not limited by any term of these terms:

- loss caused by the Developer's intent or gross negligence;
- liability for personal injury to you;
- liability that applicable law does not allow to be excluded or limited, for example product liability under EU law, and fraud;
- your statutory remedies.

Liability cannot be excluded entirely merely because you accepted these terms. The laws of the European Union and of some US states place additional limits on disclaimers and limitation-of-liability clauses; where they apply, they prevail.

## 12. Termination

- You may stop using the Software at any time and may uninstall it yourself (delete the executable and the data directory).
- If you seriously breach these terms, use the Software unlawfully, or harm the interests of third parties, the Developer may take reasonable measures to the extent the law allows, limited to matters the Developer can control, for example declining to provide support, restricting your participation in the project's official channels (the repository and the community), and reporting to the authorities or platforms concerned in accordance with law.
- Stopping use of the Software does not automatically delete your Minecraft saves, game instances or other local files. Whether they are kept is up to you.
- Provisions of these terms that by their nature should continue after termination (for example intellectual property, disclaimers and limitation of liability, and applicable law) continue after termination.

## 13. Children and minors

- These terms do not set a single worldwide minimum age.
- A minor who uses the Software must comply with the law applicable where they are and with the management requirements that their guardian lawfully imposes. Examples include the Minors Protection Law of mainland China, Article 8 of the EU GDPR and the relevant Member State rules, the US COPPA, and other applicable laws protecting minors.
- Sign-in uses a Microsoft account, and Microsoft has its own rules for minors' accounts (for example, joining a family group); where that happens the Software only shows the message returned by Microsoft.
- The Software has no built-in parental controls or play-time limits, and does not claim to have, automatically, the parental controls required in every country and region. Guardians can manage use through system settings, Microsoft Family Safety settings and similar means.
- For the position on personal information, see Section 7 of the Privacy Policy.

## 14. Regional legal terms

### 14.1 How this appendix applies

This appendix supplements or modifies these terms only where the law concerned applies to you; where it conflicts with a mandatory rule, the mandatory rule prevails (see Chapter 18). The laws listed in this appendix are a reference framework chosen by the Developer in light of the Software's actual characteristics; listing them does not mean they necessarily apply to the Software.

### 14.2 Mainland China

- **Standard terms.** These terms are standard terms drafted in advance by the provider. The Developer has, in the "Important notice" and the chapters concerned, drawn your attention in a prominent way to the terms that have a significant bearing on your interests (account and licence rules, disclaimer and limitation of liability, termination, changes to the terms, dispute resolution), and is willing to explain them at your request; please contact the Developer through the project repository (see Chapter 17). Under Articles 496 and 497 of the Civil Code, a standard term that unreasonably exempts or reduces the provider's liability, increases the other party's liability or restricts the other party's main rights is invalid; under Article 506, an exemption clause is invalid if it causes personal injury to the other party or causes property loss to the other party through intent or gross negligence. Chapters 10 and 11 of these terms are to be understood accordingly.
- **Consumer rights.** If the Consumer Rights and Interests Protection Law applies to you, your statutory rights (including Article 26 of that law on standard terms) are not reduced by these terms. The Software is currently provided free of charge and does not sell goods or paid services.
- **Contractual relationship.** These terms concern a software-use (software service) contractual relationship; the Developer does not charge you a fee.
- **Personal information and cybersecurity.** The handling of personal information is governed by the Privacy Policy. The mandatory provisions of the Personal Information Protection Law, the Cybersecurity Law (as amended in 2025, in force from 1 January 2026) and the Data Security Law are not affected by these terms.
- **Protection of minors.** The mandatory provisions of the Minors Protection Law and the Regulations on the Protection of Minors Online are not affected by these terms. The Software does not operate an online game service and does not provide game real-name verification or an anti-addiction system.

### 14.3 European Union and European Economic Area

- **GDPR.** The processing of personal data is described in the Privacy Policy.
- **Digital content and digital services directive (Directive (EU) 2019/770).** If the directive applies to the contract between you and the Developer, your statutory rights on conformity, updates and remedies are not restricted by these terms. The directive applies to contracts where the consumer pays a price or provides personal data to the trader (other than data strictly needed to supply the content or service), and it contains an exception for software supplied under a free and open-source licence, for no price, where personal data are used only to improve the software's security, compatibility or interoperability. The Software is currently provided free of charge under an open-source licence, and the Developer does not collect personal data.
- **Unfair contract terms directive (Directive 93/13/EEC).** A term that was not individually negotiated is not binding on the consumer if, contrary to the requirement of good faith, it causes a significant imbalance in the parties' rights and obligations to the consumer's detriment; the rest of the contract continues to apply. These terms are written in plain language.
- **Consumer rights directive (Directive 2011/83/EU).** The right of withdrawal mainly concerns contracts in which the consumer pays, or provides personal data to the trader. The Software is free and does not require you to provide personal data to the Developer, so withdrawal normally does not arise. If the directive applies, your right of withdrawal is not affected by these terms.
- **Digital Services Act (Regulation (EU) 2022/2065).** The Software does not host user content. If features that host user content, such as a plugin store or a community, are added later, this will be supplemented separately.

### 14.4 United States

- **Section 5 of the Federal Trade Commission Act.** The Developer will not make claims in these terms or in the Privacy Policy about functions or privacy protections that the Software does not actually have.
- **COPPA.** The Developer collects no personal information at all, including personal information of children under 13.
- **DMCA and intellectual-property complaints.** The Software does not host user content. If you believe the Software's repository or distribution page infringes your copyright or other intellectual property, please contact the Developer through the project repository, stating the work concerned, where the alleged infringement is, and how to contact you.
- **State privacy laws.** The laws of California (CCPA/CPRA), Virginia (VCDPA), Colorado (CPA), Connecticut (CTDPA), Texas (TDPSA) and other states generally set their scope by business size, the number of residents whose data is processed, or the sale of personal data. The Developer does not collect personal information. These terms do not assume that these laws all apply.
- **Consumer protection and limitation of liability.** US states have their own rules on consumer contracts and limits on software liability; where they apply, they prevail. These terms contain no mandatory arbitration clause and no class-action waiver.

## 15. Governing law and disputes

- These terms are subject to applicable law.
- These terms do not choose the law of any one jurisdiction in order to exclude the mandatory consumer-protection rules that must apply by law where you are.
- If a dispute arises, you are welcome to start by contacting the Developer through the project repository. If it cannot be resolved that way, you may seek a remedy before the courts or regulators with jurisdiction; this does not exclude the jurisdiction that the courts and regulators where you live have by law, nor your statutory remedies.
- The way disputes are resolved must be fair and lawful. These terms contain no mandatory arbitration clause and no class-action waiver.

## 16. Changes to these terms

- The Developer may update these terms where reasonable, for example because of changes in the Software's functions, in third-party services or in legal requirements.
- Each version states its version number and effective date. The latest and earlier versions are available in the project repository ([RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)), and can also be viewed in the launcher under "Settings → About".
- Changes that have a significant effect on your rights and obligations will be notified appropriately as required by applicable law, for example in the release notes of the new version or on the project page.
- Where your consent must be sought again, applicable law governs. If you do not agree to the revised terms, stop using the Software and uninstall it.

## 17. Contact information

- Developer name: RustCraftMC Team
- Project website: [rustcraftmc.github.io/cairnlauncher](https://rustcraftmc.github.io/cairnlauncher)
- Project repository: [RustCraftMC/cairn-launcher](https://github.com/RustCraftMC/cairn-launcher)
- Contact: please contact the Developer through the Issues page of the project repository. Please do not post tokens, account files or unchecked logs on public pages; for security vulnerabilities, please avoid disclosing exploitable details.

## 18. Non-waivable statutory rights

Nothing in these terms is to be interpreted as excluding, limiting or waiving any non-waivable statutory right that the User has under the law applicable where the User is.

If a term of these terms conflicts with a mandatory legal provision applicable where the User is, the applicable mandatory provision prevails to the extent of the conflict. The other valid terms continue to apply.
