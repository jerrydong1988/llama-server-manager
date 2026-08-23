#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

int main(int argc, char **argv) {
    const char *exit_after = NULL;
    const char *crash_once_marker = NULL;

    for (int index = 1; index < argc; index += 1) {
        if (strcmp(argv[index], "--exit-after-ms") == 0 && index + 1 < argc) {
            exit_after = argv[index + 1];
            index += 1;
        } else if (strcmp(argv[index], "--crash-once-marker") == 0 && index + 1 < argc) {
            crash_once_marker = argv[index + 1];
            index += 1;
        }
    }

    int should_crash = 0;
    if (crash_once_marker != NULL) {
        FILE *existing = fopen(crash_once_marker, "rb");
        if (existing != NULL) {
            fclose(existing);
        } else {
            FILE *created = fopen(crash_once_marker, "wb");
            if (created == NULL) {
                return 2;
            }
            fputs("crash once", created);
            fclose(created);
            should_crash = 1;
        }
    } else if (exit_after != NULL) {
        should_crash = 1;
    }

    if (should_crash) {
        const struct timespec delay = {0, 200000000L};
        nanosleep(&delay, NULL);
        return 1;
    }

    for (;;) {
        sleep(60);
    }
}
