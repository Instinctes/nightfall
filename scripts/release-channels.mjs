import { readFileSync } from "node:fs";

// A development preview must not silently replace the stable download channel.
// Both channels are explicit, and the active one must match the source version.
export function releaseChannels(root, version) {
    const channels = JSON.parse(readFileSync(root + "website/public/releases.json", "utf8"));
    if (!/^\d+\.\d+\.\d+$/.test(channels.stable)) throw new Error("Invalid stable release");
    if (channels.development && !/^\d+\.\d+\.\d+-dev\.\d+$/.test(channels.development)) {
        throw new Error("Invalid development release");
    }
    const active = version.includes("-") ? channels.development : channels.stable;
    if (active !== version) throw new Error(`Release channel ${active} does not match source ${version}`);
    return new Set([channels.stable, channels.development].filter(Boolean));
}

export const RELEASE_VERSION = String.raw`\d+\.\d+\.\d+(?:-dev\.\d+)?`;
