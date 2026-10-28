# Workshop scan

| item | kind | fighters | scripts | projectile scenes | size |
|---|---|---|---|---|---|
| Character Loader (2931479403) | tool / UI |  | 13 | 0 | 0.4 MB |
| Mod Options Menu (2931722541) | tool / UI |  | 15 | 0 | 0.0 MB |
| StyleEditor+ [BETA] (3037158144) | tool / UI |  | 14 | 0 | 0.3 MB |
| One Punch Man (3139650206) | character | One Punch Man | 7 | 0 | 10.2 MB |
| Satoru Gojo - The Strongest Sorcerer (3146271984) | character | Strongest | 137 | 10 | 62.5 MB |
| High-Stakes Hustler | Kinji Hakari (3270044231) | character | Hakari | 50 | 1 | 17.8 MB |
| Heion - The Strongest in History [Sukuna] (3513897396) | character | Heion | 78 | 12 | 33.8 MB |
|  (3514414430) | style |  | 0 | 0 | 0.0 MB |

## One Punch Man (3139650206)

- author Impireon, version 1.0, requires ["char_loader"]
- fighter **One Punch Man** from `OnePunchMod/Characters/OnePunchMan/One Punch Man.tscn`
- 298 files: 111 images, 32 sounds, 2 scenes, 7 scripts, 0 already imported
- scripts extend: CharacterState x3, SuperMove x2, "res://ui/CSS/CharacterSelect.gd" x1, Node x1
- 2 distinct host calls, 0 fixed, 0 Utils (10 call sites)
- everything it calls is something the official characters call too

## Satoru Gojo - The Strongest Sorcerer (3146271984)

- author Ark, version 1.1, requires ["char_loader"]
- fighter **Strongest** from `ArkStrongest/characters/Strongest/Strongest.tscn`
- 5016 files: 2282 images, 85 sounds, 108 scenes, 137 scripts, 0 already imported
- scripts extend: CharacterState x111, ObjectState x6, ThrowState x6, BaseProjectile x2, Hitbox x2, "res://characters/states/HurtAerial.gd" x1
- projectile scenes: AerialReversalRedProj.tscn, BluePulseProjectile.tscn, BlueVortexProjectile.tscn, ImaginaryHollowPurple.tscn, LapseBlueThrowProj.tscn, BlueMax.tscn, MaxBlue.tscn, OriginalHollowPurpleProj.tscn, ReversalRedHomingProj.tscn, ReversalRedProj.tscn
- 58 distinct host calls, 16 fixed, 3 Utils (1065 call sites)
- everything it calls is something the official characters call too
- would be refused by a sandbox: get_node( x10

## High-Stakes Hustler | Kinji Hakari (3270044231)

- author Chiupel (But the character itself belongs to Gege Akutami, version TEST_BUILD, requires ["char_loader"]
- fighter **Hakari** from `Hakari/HakariChar/Hakari.tscn`
- 4000 files: 1927 images, 34 sounds, 19 scenes, 50 scripts, 0 already imported
- scripts extend: CharacterState x40, ThrowState x4, "res://characters/BaseChar.gd" x1, "res://characters/states/Idle.gd" x1, "res://ui/CSS/CharacterSelect.gd" x1, BaseProjectile x1
- projectile scenes: PachinkoBall.tscn
- 25 distinct host calls, 12 fixed, 4 Utils (157 call sites)
- everything it calls is something the official characters call too

## Heion - The Strongest in History [Sukuna] (3513897396)

- author nok (fortydaynocks), version steam, requires ["char_loader"]
- fighter **Heion** from `_NokHeion/characters/heion/Heion.tscn`
- 894 files: 119 images, 66 sounds, 75 scenes, 78 scripts, 184 already imported
- scripts extend: "res://_NokHeion/characters/heion/states/HeionState.gd" x32, BaseProjectile x12, DefaultFireball x8, CharacterState x7, ActionUIData x3, Node x2
- projectile scenes: Bisect.tscn, Burrow.tscn, CursedMark.tscn, Dismantle.tscn, Dismember.tscn, Domain.tscn, Fire.tscn, Fracture.tscn, Fragment.tscn, HeionCleave.tscn, Sever.tscn, Splinter.tscn
- 65 distinct host calls, 17 fixed, 1 Utils (641 call sites)
- beyond the official scripts (host): get_node
- would be refused by a sandbox: ResourceLoader.exists x2, Steam.getSubscribedItems x1, get_node( x81, get_tree() x24

## What the official scripts cover

420 gameplay scripts use 172 distinct host calls, 29 fixed, 20 Utils.

## Calls the interpreter needs beyond the official characters

| call | mods | uses |
|---|---|---|
| host.get_node | 1 | 1 |
