import { createError, defineEventHandler, getRouterParam } from "h3";
import { $fetch } from "ofetch";

export default defineEventHandler(async (event) => {
  const id = getRouterParam(event, "id");

  if (!id) {
    throw createError({
      statusCode: 400,
      statusMessage: "Missing player id",
    });
  }

  const response = await $fetch<{ player?: any }>(
    `https://bitjita.com/api/players/${id}`,
    {
      headers: {
        "User-Agent": "BitJita (bitcraft-hub)",
        "x-app-identifier": "bitcraft-hub",
      },
    },
  ).catch(() => null);

  const player = response?.player;

  if (!player) {
    throw createError({
      statusCode: 404,
      statusMessage: "Bitjita player not found",
    });
  }

  return {
    username: (player.username as string | undefined) ?? null,
    signedIn: Boolean(
      (player.signedIn as boolean | undefined) ??
        (player.signed_in as boolean | undefined),
    ),
    lastLoginTimestamp:
      (player.lastLoginTimestamp as string | undefined) ??
      (player.last_login_timestamp as string | undefined) ??
      (player.signInTimestamp as string | undefined) ??
      (player.sign_in_timestamp as string | undefined) ??
      (player.updatedAt as string | undefined) ??
      (player.updated_at as string | undefined) ??
      null,
  };
});
