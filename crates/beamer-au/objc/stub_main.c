// Minimal stub executable for AUv3 container app.
// This exists solely to make the .app launchable, which triggers
// pluginkit to register the embedded .appex Audio Unit extension.
// The app exits immediately since it has LSBackgroundOnly=true.

int main(void) {
    return 0;
}
