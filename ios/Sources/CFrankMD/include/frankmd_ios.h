#ifndef FRANKMD_IOS_H
#define FRANKMD_IOS_H
#include <stdbool.h>

typedef struct FrankMDHandle FrankMDHandle;

FrankMDHandle *frankmd_ios_open(const char *root, const char *state);
void frankmd_ios_close(FrankMDHandle *handle);
void frankmd_ios_free_string(char *value);
char *frankmd_ios_list(FrankMDHandle *handle);
char *frankmd_ios_read(FrankMDHandle *handle, const char *path);
char *frankmd_ios_write(FrankMDHandle *handle, const char *path, const char *text);
char *frankmd_ios_create(FrankMDHandle *handle, const char *title);
char *frankmd_ios_rename(FrankMDHandle *handle, const char *old_path, const char *new_path);
bool frankmd_ios_delete(FrankMDHandle *handle, const char *path);
char *frankmd_ios_status(FrankMDHandle *handle);
bool frankmd_ios_unlock(FrankMDHandle *handle, const char *password);
void frankmd_ios_lock(FrankMDHandle *handle);
bool frankmd_ios_create_seal(FrankMDHandle *handle, const char *password);
bool frankmd_ios_import_seal(FrankMDHandle *handle, const char *json);
char *frankmd_ios_preview(FrankMDHandle *handle, const char *path);
char *frankmd_ios_search(FrankMDHandle *handle, const char *query, unsigned long limit);
bool frankmd_ios_set_encrypted(FrankMDHandle *handle, const char *path, bool encrypted);
#endif
