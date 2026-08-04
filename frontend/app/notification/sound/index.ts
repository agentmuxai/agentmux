// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Public entrypoint for the sound-notifications subsystem.
 * See docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md.
 */

export { installSoundService } from "./sound-service";
export {
    notify,
    subscribeSoundEvents,
    type SoundEvent,
} from "./sound-events";
export {
    SOUNDS,
    type SoundId,
    type SoundDef,
    type SoundCategory,
} from "./sounds";
