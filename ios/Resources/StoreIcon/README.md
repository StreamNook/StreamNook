# StoreIcon

The app icon compiled by Apple's `actool` for App Store and TestFlight builds, which refuse an icon that is not in a compiled asset catalog. `xtool dev build` does not use these files: `/root/sn-tools/testflight.sh` copies them into the app and merges `partial-Info.plist` into its `Info.plist`.

Rebuild them only when `ios/Resources/AppIcon.png` changes: put the new icon in the private `StreamNook/ios-store-icon` repo, run its "Compile iPhone icon" workflow, and replace these files with its `StoreIcon` artifact.
