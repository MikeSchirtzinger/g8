/**
 * User profile module — example G8-annotated TypeScript file.
 *
 * Demonstrates `// @g8.capability(...)` magic-comment annotations in TypeScript.
 * Run `g8 scan examples/ts-frontend` to extract these annotations.
 */

// @g8.capability(name = "profile-fetch", description = "Fetch user profile data from the API", substrate = "user-profile", consumes = ["substrate::UserId"], produces = ["substrate::UserProfile"], status = "landed")
export async function fetchUserProfile(userId: string): Promise<UserProfile> {
  const response = await fetch(`/api/users/${userId}`);
  if (!response.ok) {
    throw new ProfileError(`HTTP ${response.status} for user ${userId}`);
  }
  return response.json() as Promise<UserProfile>;
}

// @g8.capability(name = "profile-render", description = "Render a UserProfile into a display-ready ViewModel", substrate = "user-profile", consumes = ["substrate::UserProfile"], produces = ["substrate::ProfileViewModel"], status = "landed")
export function renderProfile(profile: UserProfile): ProfileViewModel {
  return {
    displayName: profile.name ?? profile.email,
    avatarUrl:
      profile.avatarUrl ??
      `https://ui-avatars.com/api/?name=${encodeURIComponent(profile.name ?? "")}`,
    joinedLabel: `Joined ${new Date(profile.createdAt).toLocaleDateString()}`,
    isVerified: profile.emailVerified,
  };
}

// @g8.capability(name = "profile-update", description = "Patch user profile fields via the API", substrate = "user-profile", consumes = ["substrate::ProfilePatch"], produces = ["substrate::UserProfile"], status = "in_flight")
export async function updateProfile(
  userId: string,
  patch: Partial<Pick<UserProfile, "name" | "avatarUrl">>
): Promise<UserProfile> {
  const response = await fetch(`/api/users/${userId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(patch),
  });
  if (!response.ok) {
    throw new ProfileError(`PATCH failed: HTTP ${response.status}`);
  }
  return response.json() as Promise<UserProfile>;
}

// ── Types ────────────────────────────────────────────────────────────────────

export interface UserProfile {
  id: string;
  name: string;
  email: string;
  avatarUrl?: string;
  emailVerified: boolean;
  createdAt: string; // ISO-8601
}

export interface ProfileViewModel {
  displayName: string;
  avatarUrl: string;
  joinedLabel: string;
  isVerified: boolean;
}

export class ProfileError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProfileError";
  }
}
