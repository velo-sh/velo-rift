/*
 * test_stat.c — VFS isolation helper for test_isolation.sh
 *
 * Calls stat() on VFS-prefixed paths to verify the inception layer
 * correctly resolves files through the manifest. When run with the shim
 * injected, stat("/vrift/<file>") is intercepted and resolved against
 * the project's manifest.
 *
 * Usage: test_stat [optional_extra_path]
 * Environment: VRIFT_VFS_PREFIX must be set (default: /vrift)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

int main(int argc, char **argv) {
  struct stat sb;
  const char *prefix = getenv("VRIFT_VFS_PREFIX");
  if (!prefix)
    prefix = "/vrift";

  /* Try both project files — isolation means only one should succeed */
  const char *files[] = {"file_a.txt", "file_b.txt"};
  int found = 0;

  for (int i = 0; i < 2; i++) {
    char path[512];
    snprintf(path, sizeof(path), "%s/%s", prefix, files[i]);

    if (stat(path, &sb) == 0) {
      printf("SUCCESS: stat(\"%s\") worked! (size=%lld, mode=0%o)\n", path,
             (long long)sb.st_size, sb.st_mode & 0777);
      found++;
    } else {
      printf(
          "INFO: stat(\"%s\") returned -1 (not in this project's manifest)\n",
          path);
    }
  }

  if (found == 0) {
    fprintf(stderr, "ERROR: No VFS files found. Is the shim loaded?\n");
    return 1;
  }

  return 0;
}
