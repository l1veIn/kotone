import type { GameProfile, InstalledGame } from "./ipc";

export function installedProfile(game: InstalledGame, profiles: GameProfile[]): GameProfile | undefined {
  const procs = game.processNames.map((p) => p.toLowerCase());
  const norm = (s: string) => s.split(/[（(]/)[0].toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
  return profiles.find((p) => p.processNames.some((name) => procs.includes(name.toLowerCase())))
    ?? profiles.find((p) => p.processNames.length > 0 && norm(p.displayName) && norm(game.name).includes(norm(p.displayName)));
}
