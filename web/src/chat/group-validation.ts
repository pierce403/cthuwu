import {
  ConsentState,
  GroupPermissionsOptions,
  PermissionPolicy,
  type GroupPermissions,
} from "@xmtp/browser-sdk";
import type { AssignmentControl } from "./control";
import { RETENTION_FROM_NS, RETENTION_IN_NS, XMTP_GROUP_MEMBER_LIMIT } from "./types";

export interface GroupForValidation {
  id: string;
  appData?: string;
  addedByInboxId?: string;
  metadata?: { creatorInboxId: string };
  members(): Promise<Array<{ inboxId: string }>>;
  listAdmins(): Promise<string[]>;
  listSuperAdmins(): Promise<string[]>;
  permissions(): Promise<GroupPermissions>;
  consentState(): Promise<ConsentState>;
  updateConsentState(state: ConsentState): Promise<void>;
  messageDisappearingSettings(): Promise<{ fromNs: bigint; inNs: bigint } | undefined>;
}

export interface ValidatedGroups {
  acolytes: GroupForValidation;
  global: Map<string, GroupForValidation>;
}

export async function validateAssignedGroups(
  assignment: AssignmentControl,
  ownInboxId: string,
  groups: Map<string, GroupForValidation>,
  expectedAgentId?: string,
): Promise<ValidatedGroups> {
  validateController(assignment, expectedAgentId);
  const acolytes = await validateAssignedAcolytesGroup(
    assignment,
    ownInboxId,
    requiredGroup(groups, assignment.acolytesGroupId),
    expectedAgentId,
  );
  const global = await validateAssignedGlobalGroups(
    assignment,
    ownInboxId,
    groups,
    expectedAgentId,
  );
  return { acolytes, global };
}

export async function validateAssignedAcolytesGroup(
  assignment: AssignmentControl,
  ownInboxId: string,
  group: GroupForValidation,
  expectedAgentId?: string,
): Promise<GroupForValidation> {
  validateController(assignment, expectedAgentId);
  await validateAcolytesGroup(group, assignment, ownInboxId);
  return group;
}

export async function validateAssignedGlobalGroups(
  assignment: AssignmentControl,
  ownInboxId: string,
  groups: Map<string, GroupForValidation>,
  expectedAgentId?: string,
): Promise<Map<string, GroupForValidation>> {
  validateController(assignment, expectedAgentId);
  const global = new Map<string, GroupForValidation>();
  for (const id of assignment.global.readConversationIds) {
    const group = requiredGroup(groups, id);
    await validateGlobalGroup(group, assignment, ownInboxId);
    global.set(id, group);
  }
  return global;
}

function validateController(assignment: AssignmentControl, expectedAgentId?: string): void {
  if (expectedAgentId && expectedAgentId !== assignment.tentacleAgentId) {
    throw new Error("assignment controller does not match canonical Branding state");
  }
}

async function validateAcolytesGroup(
  group: GroupForValidation,
  assignment: AssignmentControl,
  ownInboxId: string,
): Promise<void> {
  if (group.id !== assignment.acolytesGroupId) throw new Error("Acolytes conversation ID mismatch");
  const data = parseAppData(group.appData);
  if (
    !hasExactKeys(data, [
      "app",
      "channel",
      "environment",
      "tentacleAgentId",
      "tentacleInboxId",
      "version",
    ]) ||
    data.app !== "cthuwu.chat" ||
    data.version !== 1 ||
    data.environment !== "production" ||
    data.channel !== "acolytes" ||
    data.tentacleAgentId !== assignment.tentacleAgentId ||
    data.tentacleInboxId !== assignment.tentacleInboxId
  ) {
    throw new Error("Acolytes appData is not the exact supported schema");
  }
  await validateMembership(group, ownInboxId, assignment.tentacleInboxId);
  if (
    group.addedByInboxId !== assignment.tentacleInboxId ||
    group.metadata?.creatorInboxId !== assignment.tentacleInboxId
  ) {
    throw new Error("Acolytes group was not created and added by the assigned Tentacle");
  }
  await validateAcolytesAdmins(group, assignment.tentacleInboxId);
  await validateAdminOnlyPolicy(group);
  await validateRetention(group);
}

async function validateGlobalGroup(
  group: GroupForValidation,
  assignment: AssignmentControl,
  ownInboxId: string,
): Promise<void> {
  if (!assignment.global.readConversationIds.includes(group.id)) {
    throw new Error("Global conversation ID is not in the trusted binding");
  }
  const data = parseAppData(group.appData);
  if (
    !hasExactKeys(data, [
      "app",
      "channel",
      "environment",
      "logicalChannelId",
      "shardId",
      "version",
    ]) ||
    data.app !== "cthuwu.chat" ||
    data.version !== 1 ||
    data.environment !== "production" ||
    data.channel !== "global" ||
    data.logicalChannelId !== assignment.global.logicalChannelId ||
    data.shardId !== "primary"
  ) {
    throw new Error("Global appData is not the exact supported schema");
  }
  await validateMembership(group, ownInboxId, assignment.tentacleInboxId);
  await validateAdmins(group, assignment.global.adminInboxIds);
  await validateAdminOnlyPolicy(group);
  await validateRetention(group);
}

async function validateAcolytesAdmins(
  group: GroupForValidation,
  tentacleInboxId: string,
): Promise<void> {
  const admins = await group.listAdmins();
  const superAdmins = await group.listSuperAdmins();
  if (admins.length !== 0 || !sameSet(superAdmins, [tentacleInboxId])) {
    throw new Error("Acolytes group must have the assigned Tentacle as its sole super-admin");
  }
}

async function validateAdminOnlyPolicy(group: GroupForValidation): Promise<void> {
  const permissions = await group.permissions();
  const policy = permissions.policySet;
  if (
    permissions.policyType !== GroupPermissionsOptions.AdminOnly ||
    policy.addMemberPolicy !== PermissionPolicy.Admin ||
    policy.removeMemberPolicy !== PermissionPolicy.Admin ||
    policy.addAdminPolicy !== PermissionPolicy.SuperAdmin ||
    policy.removeAdminPolicy !== PermissionPolicy.SuperAdmin ||
    policy.updateGroupNamePolicy !== PermissionPolicy.Admin ||
    policy.updateGroupDescriptionPolicy !== PermissionPolicy.Admin ||
    policy.updateGroupImageUrlSquarePolicy !== PermissionPolicy.Admin ||
    policy.updateMessageDisappearingPolicy !== PermissionPolicy.Admin ||
    policy.updateAppDataPolicy !== PermissionPolicy.Admin
  ) {
    throw new Error("group does not use the pinned XMTP AdminOnly permission policy");
  }
}

async function validateMembership(
  group: GroupForValidation,
  ownInboxId: string,
  tentacleInboxId: string,
): Promise<void> {
  const members = await group.members();
  if (members.length < 2 || members.length > XMTP_GROUP_MEMBER_LIMIT) {
    throw new Error("group membership exceeds the supported XMTP v1 bounds");
  }
  const ids = members.map((member) => member.inboxId);
  if (new Set(ids).size !== ids.length || !ids.includes(ownInboxId) || !ids.includes(tentacleInboxId)) {
    throw new Error("group does not contain the authenticated acolyte and assigned Tentacle");
  }
}

async function validateAdmins(group: GroupForValidation, expected: string[]): Promise<void> {
  const actual = [...new Set([...(await group.listAdmins()), ...(await group.listSuperAdmins())])].sort();
  if (!sameSet(actual, expected)) {
    throw new Error("group admin set does not match the trusted assignment");
  }
}

function sameSet(actual: readonly string[], expected: readonly string[]): boolean {
  const left = [...new Set(actual)].sort();
  const right = [...new Set(expected)].sort();
  return left.length === right.length && left.every((id, index) => id === right[index]);
}

async function validateRetention(group: GroupForValidation): Promise<void> {
  const policy = await group.messageDisappearingSettings();
  if (policy?.fromNs !== RETENTION_FROM_NS || policy.inNs !== RETENTION_IN_NS) {
    throw new Error("group does not enforce the required 14-day message policy");
  }
}

function requiredGroup(
  groups: Map<string, GroupForValidation>,
  id: string,
): GroupForValidation {
  const group = groups.get(id);
  if (!group || group.id !== id) throw new Error("assigned XMTP group is not available");
  return group;
}

function parseAppData(value: string | undefined): Record<string, unknown> {
  if (!value || value.length > 2_048) throw new Error("group appData is missing or too large");
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    throw new Error("group appData is malformed");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("group appData is malformed");
  }
  return parsed as Record<string, unknown>;
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}
