import {
  ConsentState,
  GroupPermissionsOptions,
  PermissionPolicy,
} from "@xmtp/browser-sdk";
import { describe, expect, it } from "vitest";
import type { AssignmentControl } from "./control";
import { validateAssignedGroups, type GroupForValidation } from "./group-validation";

const own = "1".repeat(64);
const tentacle = "2".repeat(64);
const acolytesId = "3".repeat(64);
const globalId = "4".repeat(64);
const adminOnlyPolicy = {
  addMemberPolicy: PermissionPolicy.Admin,
  removeMemberPolicy: PermissionPolicy.Admin,
  addAdminPolicy: PermissionPolicy.SuperAdmin,
  removeAdminPolicy: PermissionPolicy.SuperAdmin,
  updateGroupNamePolicy: PermissionPolicy.Admin,
  updateGroupDescriptionPolicy: PermissionPolicy.Admin,
  updateGroupImageUrlSquarePolicy: PermissionPolicy.Admin,
  updateMessageDisappearingPolicy: PermissionPolicy.Admin,
  updateAppDataPolicy: PermissionPolicy.Admin,
};
const assignment: AssignmentControl = {
  type: "cthuwu.assignment.v1",
  requestId: "5".repeat(32),
  environment: "production",
  revision: `123:0x${"6".repeat(64)}`,
  tentacleAgentId: "42",
  tentacleInboxId: tentacle,
  acolytesGroupId: acolytesId,
  global: {
    logicalChannelId: "cthuwu.global.v1",
    readConversationIds: [globalId],
    writeConversationId: globalId,
    adminInboxIds: [tentacle],
  },
  retention: { fromNs: "1", inNs: "1209600000000000" },
};

function group(id: string, channel: "acolytes" | "global"): GroupForValidation {
  return {
    id,
    addedByInboxId: tentacle,
    metadata: { creatorInboxId: tentacle },
    appData: JSON.stringify(channel === "acolytes" ? {
      app: "cthuwu.chat",
      version: 1,
      environment: "production",
      channel,
      tentacleAgentId: "42",
      tentacleInboxId: tentacle,
    } : {
      app: "cthuwu.chat",
      version: 1,
      environment: "production",
      channel,
      logicalChannelId: "cthuwu.global.v1",
      shardId: "primary",
    }),
    members: async () => [{ inboxId: own }, { inboxId: tentacle }],
    listAdmins: async () => channel === "global" ? [tentacle] : [],
    listSuperAdmins: async () => channel === "acolytes" ? [tentacle] : [],
    permissions: async () => ({
      policyType: GroupPermissionsOptions.AdminOnly,
      policySet: { ...adminOnlyPolicy },
    }),
    consentState: async () => ConsentState.Allowed,
    updateConsentState: async () => undefined,
    messageDisappearingSettings: async () => ({ fromNs: 1n, inNs: 1_209_600_000_000_000n }),
  };
}

describe("trusted assignment group validation", () => {
  it("accepts exact IDs, appData, members, admins, and retention", async () => {
    const groups = new Map([
      [acolytesId, group(acolytesId, "acolytes")],
      [globalId, group(globalId, "global")],
    ]);
    await expect(validateAssignedGroups(assignment, own, groups, "42")).resolves.toMatchObject({
      acolytes: { id: acolytesId },
    });
  });

  it("rejects spoofed names/appData, admins, controller IDs, and policies", async () => {
    const spoofed = group(acolytesId, "acolytes");
    spoofed.appData = JSON.stringify({ name: "Acolytes" });
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, spoofed], [globalId, group(globalId, "global")]]),
      "42",
    )).rejects.toThrow(/appData/u);

    const wrongAdmins = group(globalId, "global");
    wrongAdmins.listAdmins = async () => [tentacle, "7".repeat(64)];
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, group(acolytesId, "acolytes")], [globalId, wrongAdmins]]),
      "42",
    )).rejects.toThrow(/admin/u);
    await expect(validateAssignedGroups(assignment, own, new Map(), "43")).rejects.toThrow(/Branding/u);

    const wrongPolicy = group(acolytesId, "acolytes");
    wrongPolicy.messageDisappearingSettings = async () => ({ fromNs: 0n, inNs: 1n });
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, wrongPolicy], [globalId, group(globalId, "global")]]),
      "42",
    )).rejects.toThrow(/14-day/u);

    const wrongCreator = group(acolytesId, "acolytes");
    wrongCreator.metadata = { creatorInboxId: "8".repeat(64) };
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, wrongCreator], [globalId, group(globalId, "global")]]),
      "42",
    )).rejects.toThrow(/created and added/u);

    const wrongAdder = group(acolytesId, "acolytes");
    wrongAdder.addedByInboxId = "8".repeat(64);
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, wrongAdder], [globalId, group(globalId, "global")]]),
      "42",
    )).rejects.toThrow(/created and added/u);

    const permissive = group(globalId, "global");
    permissive.permissions = async () => ({
      policyType: GroupPermissionsOptions.Default,
      policySet: { ...adminOnlyPolicy, addMemberPolicy: PermissionPolicy.Allow },
    });
    await expect(validateAssignedGroups(
      assignment,
      own,
      new Map([[acolytesId, group(acolytesId, "acolytes")], [globalId, permissive]]),
      "42",
    )).rejects.toThrow(/AdminOnly/u);
  });
});
