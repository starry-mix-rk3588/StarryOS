

#include <stdio.h>

typedef  unsigned long ulong;

struct drm_version { int major; int minor; int patchlevel; ulong name_len; char *name; ulong date_len; char *date; ulong desc_len; char *desc; };

int main() {

    printf("sizeof: %d\n", sizeof(struct drm_version));

    return 0;
}