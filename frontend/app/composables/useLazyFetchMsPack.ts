import { unpack } from "msgpackr/unpack";

export function useLazyFetchMsPack<DataT, ErrorT = undefined>(
  ...args: Parameters<typeof useLazyFetch<DataT, ErrorT>>
): ReturnType<typeof useLazyFetch<DataT, ErrorT>> {
  const {
    public: { api },
  } = useRuntimeConfig();
  const [request, options] = args;

  const baseURL = (() => {
    const configured = (api.base ?? "").trim();
    if (!configured) {
      return undefined;
    }

    if (import.meta.client) {
      const hostname = window.location.hostname;
      const isLocalHost = hostname === "localhost" || hostname === "127.0.0.1";
      const pointsToLocalhost =
        configured.includes("localhost") || configured.includes("127.0.0.1");

      if (!isLocalHost && pointsToLocalhost) {
        return undefined;
      }
    }

    return configured;
  })();

  return useLazyFetch<DataT, ErrorT>(request, {
    baseURL,
    ...options,
    headers: {
      Accept: "application/vnd.msgpack",
    },
    transform: async (response: Blob) => {
      try {
        return unpack((await response.arrayBuffer()) as Buffer, {
          int64AsType: "auto",
        });
      } catch (e) {
        console.error("msgpack Parsing Error:", e);
        throw new Error("Failed to parse msgpack response.");
      }
    },
  });
}
