#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

static void (*sendAction)(const char *);
static __weak NSMenu *previewMenu;
static NSDictionary *smokeAction;
static void captureSmokeAction(const char *json) { smokeAction=[NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil]; }
static NSString *str(id value) { return value && value != NSNull.null ? [value description] : @""; }
static BOOL yes(id value) { return [value respondsToSelector:@selector(boolValue)] && [value boolValue]; }
static NSArray *array(id value) { return [value isKindOfClass:NSArray.class] ? value : @[]; }
static NSDictionary *dict(id value) { return [value isKindOfClass:NSDictionary.class] ? value : @{}; }
static void sendPayload(NSDictionary *payload) {
    NSData *data=[NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
    NSString *json=[[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    if (sendAction && json) sendAction(json.UTF8String);
}
@interface MCStackView : NSStackView
@end
@implementation MCStackView
- (BOOL)isFlipped { return YES; }
@end
@interface MCButton : NSButton
@property NSDictionary *payload;
@end
@implementation MCButton
@end
@interface MCController : NSObject<NSWindowDelegate>
@property NSWindow *window;
@property NSScrollView *scroll;
@property NSStackView *content;
@property NSDictionary *state;
@property NSSegmentedControl *tabs;
@property NSInteger selectedTab;
@property NSTextField *cidr;
@property NSPopUpButton *theme;
@property NSPopUpButton *titleMode;
@property NSPopUpButton *iconColor;
@property NSMutableArray<MCButton *> *fields;
@property NSMutableArray<MCButton *> *interfaces;
@property NSMutableArray<MCButton *> *traySections;
@property NSMutableArray<MCButton *> *hiddenSections;
@end
static MCController *controller;
static NSTextField *label(NSString *text, CGFloat size, BOOL bold) {
    NSTextField *view=[NSTextField wrappingLabelWithString:text];
    view.font=bold ? [NSFont systemFontOfSize:size weight:NSFontWeightSemibold] : [NSFont systemFontOfSize:size];
    view.selectable=YES;
    return view;
}
static NSStackView *stack(NSUserInterfaceLayoutOrientation orientation) {
    NSStackView *view=[MCStackView new];view.orientation=orientation;view.spacing=12;view.alignment=orientation==NSUserInterfaceLayoutOrientationVertical ? NSLayoutAttributeLeading : NSLayoutAttributeCenterY;
    return view;
}
@implementation MCController
- (instancetype)init {
    if (!(self=[super init])) return nil;
    self.window=[[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,1080,780) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
    self.window.backgroundColor=NSColor.windowBackgroundColor;self.window.title=@"MikroCockpit";self.window.minSize=NSMakeSize(860,560);self.window.releasedWhenClosed=NO;self.window.delegate=self;
    [self.window center];
    NSView *root=self.window.contentView;
    self.tabs=[NSSegmentedControl segmentedControlWithLabels:@[@"Overview",@"Devices",@"Routes & VPN",@"Exceptions",@"Customize"] trackingMode:NSSegmentSwitchTrackingSelectOne target:self action:@selector(tabChanged:)];
    self.tabs.segmentStyle=NSSegmentStyleSeparated;self.tabs.selectedSegment=0;self.tabs.translatesAutoresizingMaskIntoConstraints=NO;[root addSubview:self.tabs];
    self.scroll=[NSScrollView new];self.scroll.hasVerticalScroller=YES;self.scroll.drawsBackground=NO;self.scroll.translatesAutoresizingMaskIntoConstraints=NO;[root addSubview:self.scroll];
    [NSLayoutConstraint activateConstraints:@[[self.tabs.topAnchor constraintEqualToAnchor:root.topAnchor constant:18],[self.tabs.centerXAnchor constraintEqualToAnchor:root.centerXAnchor],[self.scroll.topAnchor constraintEqualToAnchor:self.tabs.bottomAnchor constant:18],[self.scroll.leadingAnchor constraintEqualToAnchor:root.leadingAnchor],[self.scroll.trailingAnchor constraintEqualToAnchor:root.trailingAnchor],[self.scroll.bottomAnchor constraintEqualToAnchor:root.bottomAnchor]]];
    return self;
}
- (void)windowWillClose:(NSNotification *)notification { controller=nil; }
- (void)tabChanged:(id)sender { self.selectedTab=self.tabs.selectedSegment;[self.scroll.contentView scrollToPoint:NSZeroPoint];[self render]; }
- (MCButton *)button:(NSString *)title payload:(NSDictionary *)payload enabled:(BOOL)enabled {
    MCButton *b=[MCButton buttonWithTitle:title target:self action:@selector(pressed:)];b.payload=payload;b.enabled=enabled;b.bezelStyle=NSBezelStyleTexturedRounded;return b;
}
- (void)pressed:(MCButton *)sender { sendPayload(sender.payload); }
- (void)add:(NSView *)view to:(NSStackView *)parent {
    [parent addArrangedSubview:view];
    if ([view isKindOfClass:NSStackView.class]) [view.widthAnchor constraintEqualToAnchor:parent.widthAnchor constant:parent==self.content ? -48 : -28].active=YES;
}
- (NSStackView *)card:(NSString *)title {
    NSStackView *v=stack(NSUserInterfaceLayoutOrientationVertical);v.edgeInsets=NSEdgeInsetsMake(16,14,16,14);v.wantsLayer=YES;v.layer.cornerRadius=12;v.layer.backgroundColor=NSColor.controlBackgroundColor.CGColor;v.layer.borderColor=NSColor.separatorColor.CGColor;v.layer.borderWidth=.5;
    [self add:v to:self.content];[v addArrangedSubview:label(title,19,YES)];return v;
}
- (void)directButtons:(NSString *)address to:(NSStackView *)v ready:(BOOL)ready {
    NSStackView *row=stack(NSUserInterfaceLayoutOrientationHorizontal);
    for (NSArray *option in @[@[@"Direct · 15 min",@15],@[@"Direct · 1 hour",@60],@[@"Direct · permanent",NSNull.null]]) {
        [row addArrangedSubview:[self button:option[0] payload:@{@"kind":@"direct",@"address":address,@"minutes":option[1]} enabled:ready]];
    }
    [row addArrangedSubview:[self button:@"Return to VPN" payload:@{@"kind":@"end_direct",@"address":address} enabled:ready]];
    [v addArrangedSubview:row];
}
- (void)render {
    NSDictionary *s=dict(self.state[@"snapshot"]),*c=dict(self.state[@"config"]),*p=dict(self.state[@"preferences"]);
    NSString *theme=str(p[@"theme"]);self.window.appearance=[theme isEqual:@"system"] ? nil : [NSAppearance appearanceNamed:[theme isEqual:@"light"] ? NSAppearanceNameAqua : NSAppearanceNameDarkAqua];
    NSPoint offset=self.scroll.contentView.bounds.origin;
    self.content=stack(NSUserInterfaceLayoutOrientationVertical);self.content.edgeInsets=NSEdgeInsetsMake(20,24,32,24);self.content.translatesAutoresizingMaskIntoConstraints=NO;
    self.scroll.documentView=self.content;[self.content.widthAnchor constraintEqualToAnchor:self.scroll.contentView.widthAnchor].active=YES;
    BOOL ready=yes(self.state[@"policy_ready"]),blocked=yes(self.state[@"busy"])||self.state[@"error"]!=NSNull.null && self.state[@"error"]!=nil;
    NSStackView *header=stack(NSUserInterfaceLayoutOrientationHorizontal);[header addArrangedSubview:label(@"Your network. Your rules.",28,YES)];[header addArrangedSubview:[self button:@"Refresh" payload:@{@"kind":@"refresh"} enabled:!yes(self.state[@"busy"])]];[self add:header to:self.content];
    [self.content addArrangedSubview:label(str(c[@"name"]),14,NO)];
    NSTextField *notice=label(str(self.state[@"error"]!=NSNull.null && self.state[@"error"] ? self.state[@"error"] : self.state[@"message"]),13,NO);notice.textColor=blocked ? NSColor.systemOrangeColor : NSColor.secondaryLabelColor;[self.content addArrangedSubview:notice];
    if (self.selectedTab==4) { [self preferences:p snapshot:s]; }
    else if (!s.count) { [self.content addArrangedSubview:label(@"Connecting to router…",18,NO)]; }
    else if (self.selectedTab==0) {
        NSStackView *hero=[self card:@"Network status"];
        NSStackView *statusRow=stack(NSUserInterfaceLayoutOrientationHorizontal);
        NSString *kind=str(self.state[@"icon_state"]);
        if([@[@"online",@"offline",@"vpn"] containsObject:kind]) {
            NSString *imagePath=[NSBundle.mainBundle pathForResource:[kind stringByAppendingString:@"-large@2x"] ofType:@"png"];
            NSImage *image=imagePath ? [[NSImage alloc] initWithContentsOfFile:imagePath] : nil;
            if(image) { NSImageView *iv=[NSImageView imageViewWithImage:image];[iv.widthAnchor constraintEqualToConstant:88].active=YES;[iv.heightAnchor constraintEqualToConstant:88].active=YES;[statusRow addArrangedSubview:iv]; }
        }
        [statusRow addArrangedSubview:label(str(self.state[@"status_line"]),20,YES)];[hero addArrangedSubview:statusRow];
        [hero addArrangedSubview:label(@"Internet checks run from this Mac. The VPN indicator describes the selected router default; device and destination exceptions may apply.",12,NO)];
        NSString *wgAddresses=[array(s[@"wg_addresses"]) componentsJoinedByString:@", "];
        [hero addArrangedSubview:label([NSString stringWithFormat:@"WG local IP: %@",wgAddresses.length ? wgAddresses : @"unavailable"],14,YES)];
        NSString *wan=[array(s[@"wan_addresses"]) componentsJoinedByString:@", "];
        [hero addArrangedSubview:label([NSString stringWithFormat:@"WAN IP: %@",wan.length ? wan : @"unavailable"],14,YES)];
        [hero addArrangedSubview:label([NSString stringWithFormat:@"VPN public IP: %@",[s[@"public_ip"] isKindOfClass:NSString.class] ? s[@"public_ip"] : @"unavailable"],14,YES)];
        [hero addArrangedSubview:label(@"VPN public IP is measured on the router. WAN IP is its interface address and may be behind NAT.",12,NO)];
        NSString *probeDetail=str(s[@"public_ip_source"]);if (!probeDetail.length) probeDetail=str(s[@"public_ip_error"]);
        if (probeDetail.length) [hero addArrangedSubview:label(probeDetail,12,NO)];
        NSDictionary *resource=dict(s[@"resource"]);
        [hero addArrangedSubview:label([NSString stringWithFormat:@"RouterOS %@    CPU %@%%    Free memory %.0f MB    Uptime %@",str(resource[@"version"]),str(resource[@"cpu-load"]),[resource[@"free-memory"] doubleValue]/1048576.0,str(resource[@"uptime"])],14,NO)];
        [hero addArrangedSubview:label(ready ? @"Protection policy verified" : @"Protection policy not installed — direct access controls are unavailable",13,NO)];
        NSStackView *direct=[self card:@"Entire LAN"];
        [direct addArrangedSubview:label(@"IPv4 direct access for testing. IPv6 stays blocked by the installed policy.",13,NO)];[self directButtons:@"all" to:direct ready:ready&&!blocked];
        NSStackView *ports=[self card:@"Interfaces"];
        for (NSDictionary *i in array(s[@"interfaces"])) {
            NSArray *rate=dict(s[@"rates"])[str(i[@"name"])];
            NSString *speed=rate ? [NSString stringWithFormat:@"↓ %.2f   ↑ %.2f Mbit/s",[rate[0] doubleValue]/1000000,[rate[1] doubleValue]/1000000] : @"Measuring…";
            [ports addArrangedSubview:label([NSString stringWithFormat:@"%@  %@    %@",yes(i[@"running"])&&!yes(i[@"disabled"]) ? @"🟢" : @"🔴",str(i[@"name"]),speed],13,NO)];
        }
    } else if (self.selectedTab==1) {
        for (NSDictionary *device in array(s[@"leases"])) {
            NSString *ip=str(device[@"address"]),*name=str(device[@"host-name"]);NSStackView *v=[self card:[NSString stringWithFormat:@"%@  ·  %@",name.length?name:@"Device",ip]];
            NSMutableArray *exceptions=[NSMutableArray new];for(NSDictionary *entry in array(s[@"direct"])) if([str(entry[@"address"]) isEqual:ip]||[str(entry[@"address"]) isEqual:[ip stringByAppendingString:@"/32"]]) [exceptions addObject:str(entry[@"timeout"]).length?str(entry[@"timeout"]):@"Permanent"];
            [v addArrangedSubview:label(exceptions.count ? [@"Direct exception: " stringByAppendingString:[exceptions componentsJoinedByString:@", "]] : @"Router policy (LAN and destination exceptions may also apply)",13,NO)];
            [self directButtons:ip to:v ready:ready&&!blocked];
        }
    } else if (self.selectedTab==2) {
        NSStackView *exits=[self card:@"VPN / proxy exit"];
        NSDictionary *group=dict(dict(s[@"proxies"])[str(c[@"group"])]);
        for (NSString *name in array(group[@"all"])) {
            if ([@[@"DIRECT",@"REJECT",@"PASS"] containsObject:name]) continue;
            [exits addArrangedSubview:[self button:[NSString stringWithFormat:@"%@ %@",[name isEqual:group[@"now"]]?@"✓":@"",name] payload:@{@"kind":@"exit",@"name":name} enabled:!blocked]];
        }
        NSStackView *routes=[self card:[@"Internet VPN · " stringByAppendingString:str(c[@"table"])]];
        for (NSDictionary *r in array(s[@"routes"])) {
            if (![str(r[@"dst-address"]) isEqual:@"0.0.0.0/0"]) continue;
            NSStackView *row=stack(NSUserInterfaceLayoutOrientationHorizontal);
            NSString *name=str(r[@"comment"]).length ? str(r[@"comment"]) : str(r[@"gateway"]);
            BOOL primary=yes(r[@"active"])&&[str(r[@"distance"]) isEqual:@"1"];
            [row addArrangedSubview:label([NSString stringWithFormat:@"%@ %@   distance %@",primary?@"●":@"○",name,str(r[@"distance"])],13,NO)];
            [row addArrangedSubview:[self button:@"Make primary" payload:@{@"kind":@"primary",@"id":str(r[@".id"])} enabled:ready&&!blocked&&!yes(r[@"disabled"])&&!yes(r[@"dynamic"])&&[str(r[@"dst-address"]) isEqual:@"0.0.0.0/0"]]];[routes addArrangedSubview:row];
        }
        [routes addArrangedSubview:label(ready ? @"Kill switch verified · no direct WAN fallback" : @"Kill switch not verified · switching disabled",12,NO)];
        NSStackView *wg=[self card:@"WireGuard"];
        for (NSDictionary *w in array(s[@"wg"])) {
            NSString *name=str(w[@"name"]);BOOL allowed=[array(c[@"wireguard"]) containsObject:name]&&![array(c[@"protected"]) containsObject:name];
            [wg addArrangedSubview:[self button:[NSString stringWithFormat:@"%@ · %@",name,allowed ? (yes(w[@"disabled"])?@"Enable":@"Disable") : @"Protected / read only"] payload:@{@"kind":@"wireguard",@"name":name,@"enabled":@(!yes(w[@"disabled"])?NO:YES)} enabled:allowed&&!blocked]];
        }
    } else {
        NSStackView *v=[self card:@"Destination exceptions"];
        [v addArrangedSubview:label(@"Direct access for the entire LAN. Only exceptions owned by MikroCockpit are listed.",13,NO)];
        NSStackView *row=stack(NSUserInterfaceLayoutOrientationHorizontal);self.cidr=[NSTextField new];self.cidr.placeholderString=@"IPv4 address or subnet";[self.cidr.widthAnchor constraintEqualToConstant:320].active=YES;[row addArrangedSubview:self.cidr];NSButton *add=[NSButton buttonWithTitle:@"Add exception" target:self action:@selector(addBypass:)];add.enabled=!blocked;[row addArrangedSubview:add];[v addArrangedSubview:row];
        for (NSDictionary *r in array(s[@"bypass"])) { [v addArrangedSubview:[self button:[@"Remove " stringByAppendingString:str(r[@"address"])] payload:@{@"kind":@"remove_bypass",@"id":str(r[@".id"]),@"address":str(r[@"address"])} enabled:!blocked]]; }
    }
    [self.content layoutSubtreeIfNeeded];[self.scroll.contentView scrollToPoint:offset];[self.scroll reflectScrolledClipView:self.scroll.contentView];
}
- (void)addBypass:(id)sender { sendPayload(@{@"kind":@"add_bypass",@"address":self.cidr.stringValue}); }
- (MCButton *)check:(NSString *)title key:(NSString *)key selected:(BOOL)selected to:(NSStackView *)v {
    MCButton *b=[MCButton checkboxWithTitle:title target:nil action:nil];b.payload=@{@"key":key};b.state=selected?NSControlStateValueOn:NSControlStateValueOff;[v addArrangedSubview:b];return b;
}
- (void)preferences:(NSDictionary *)p snapshot:(NSDictionary *)s {
    NSStackView *v=[self card:@"Appearance & tray"];
    self.theme=[NSPopUpButton new];[self.theme addItemsWithTitles:@[@"dark",@"light",@"system"]];[self.theme selectItemWithTitle:str(p[@"theme"])];[v addArrangedSubview:label(@"Theme",13,YES)];[v addArrangedSubview:self.theme];
    self.iconColor=[NSPopUpButton new];[self.iconColor addItemsWithTitles:@[@"system",@"monochrome",@"blue",@"purple",@"green",@"orange",@"pink"]];[self.iconColor selectItemWithTitle:[p[@"icon_color"] isKindOfClass:NSString.class] ? p[@"icon_color"] : @"system"];[v addArrangedSubview:label(@"Menu icon color (system = macOS accent)",13,YES)];[v addArrangedSubview:self.iconColor];
    self.titleMode=[NSPopUpButton new];[self.titleMode addItemsWithTitles:@[@"custom",@"exit",@"cpu",@"name",@"icon"]];[self.titleMode selectItemWithTitle:str(p[@"tray_title"])];[v addArrangedSubview:label(@"Text to the right of the tray icon",13,YES)];[v addArrangedSubview:self.titleMode];
    self.fields=[NSMutableArray new];
    for (NSArray *field in @[@[@"internet",@"Internet reachability"],@[@"connection",@"Interface link up/down"],@[@"speed",@"Download / upload"],@[@"exit",@"Selected exit"],@[@"enabled",@"Enabled tunnels / proxy"],@[@"interfaces",@"Interface lights"],@[@"cpu",@"CPU"],@[@"uptime",@"Uptime"],@[@"wan_ip",@"WAN IP"],@[@"wg_ip",@"WireGuard local IP"],@[@"public_ip",@"VPN public IP"]]) [self.fields addObject:[self check:field[1] key:field[0] selected:[array(p[@"status_fields"]) containsObject:field[0]] to:v]];
    [v addArrangedSubview:label(@"Interfaces (empty = profile WAN)",13,YES)];self.interfaces=[NSMutableArray new];
    for(NSDictionary *i in array(s[@"interfaces"])) [self.interfaces addObject:[self check:str(i[@"name"]) key:str(i[@"name"]) selected:[array(p[@"status_interfaces"]) containsObject:i[@"name"]] to:v]];
    [v addArrangedSubview:label(@"Tray sections (retains saved order)",13,YES)];self.traySections=[NSMutableArray new];
    NSMutableArray *order=[array(p[@"tray_order"]) mutableCopy];for(NSString *key in @[@"direct",@"exits",@"routes",@"wireguard",@"bypass"]) if(![order containsObject:key]) [order addObject:key];
    for(NSString *key in order) [self.traySections addObject:[self check:key key:key selected:[array(p[@"tray_order"]) containsObject:key] to:v]];
    NSButton *save=[NSButton buttonWithTitle:@"Save preferences" target:self action:@selector(savePreferences:)];[v addArrangedSubview:save];
}
- (NSArray *)selected:(NSArray<MCButton *> *)buttons {
    NSMutableArray *keys=[NSMutableArray new];for(MCButton *b in buttons) if(b.state==NSControlStateValueOn) [keys addObject:b.payload[@"key"]];return keys;
}
- (void)savePreferences:(id)sender {
    sendPayload(@{@"kind":@"preferences",@"value":@{@"theme":self.theme.titleOfSelectedItem,@"icon_color":self.iconColor.titleOfSelectedItem,@"hidden":array(dict(self.state[@"preferences"])[@"hidden"]),@"tray_order":[self selected:self.traySections],@"tray_title":self.titleMode.titleOfSelectedItem,@"status_fields":[self selected:self.fields],@"status_interfaces":[self selected:self.interfaces]}});
    self.selectedTab=0;self.tabs.selectedSegment=0;
}
@end
void cockpit_dashboard(const char *json, bool show, void (*callback)(const char *)) {
    @autoreleasepool {
        sendAction=callback;
        if (!controller && !show) return;
        if(!controller) controller=[MCController new];sendAction=callback;
        NSData *data=[[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
        NSDictionary *value=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];if(![value isKindOfClass:NSDictionary.class]) return;
        controller.state=value;
        // Preserve edits until the user saves or leaves the preferences/exception tab.
        if(show || (controller.selectedTab!=4 && controller.selectedTab!=3)) [controller render];
        if(show) { [NSApp activateIgnoringOtherApps:YES];[controller.window makeKeyAndOrderFront:nil]; }
    }
}
void cockpit_capture(const char *path) {
    for(NSInteger tab=0;tab<5;tab++) {
        controller.selectedTab=tab;controller.tabs.selectedSegment=tab;[controller.scroll.contentView scrollToPoint:NSZeroPoint];[controller render];
        NSView *view=controller.window.contentView;[view layoutSubtreeIfNeeded];
        NSBitmapImageRep *rep=[view bitmapImageRepForCachingDisplayInRect:view.bounds];
        [view.effectiveAppearance performAsCurrentDrawingAppearance:^{ [view cacheDisplayInRect:view.bounds toBitmapImageRep:rep]; }];
        NSString *file=[NSString stringWithFormat:@"%@-%ld.png",[[NSString stringWithUTF8String:path] stringByDeletingPathExtension],(long)tab];
        [[rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:file atomically:YES];
    }
    [controller.window close];
    NSCAssert(controller==nil, @"Closing the dashboard must release its controller");
    if (getenv("MIKROCOCKPIT_MENU_PREVIEW")) {
        NSMenu *menu=previewMenu;
        NSCAssert([menu itemWithTitle:@"Open dashboard…"].image != nil, @"Menu icon must load");
        for (NSString *name in @[@"Direct access switch",@"This Mac access switch"])
        for (NSView *view in [menu itemWithTitle:name].view.subviews) if ([view isKindOfClass:NSSwitch.class]) {
            NSControl *toggle=(NSControl *)view;BOOL enabled=toggle.enabled;toggle.enabled=YES;
            void (*callback)(const char *)=sendAction;sendAction=captureSmokeAction;
            [toggle sendAction:toggle.action to:toggle.target];sendAction=callback;toggle.enabled=enabled;
            NSCAssert([smokeAction[@"kind"] isEqual:@"direct"] && [smokeAction[@"address"] isEqual:[name isEqual:@"This Mac access switch"] ? @"192.168.88.10" : @"all"] && [smokeAction[@"minutes"] integerValue]==15, @"Switch must request timed direct access");
            smokeAction=nil;
        }
        NSTimer *timer=[NSTimer timerWithTimeInterval:8 repeats:NO block:^(NSTimer *t){[menu cancelTracking];}];
        [NSRunLoop.mainRunLoop addTimer:timer forMode:NSRunLoopCommonModes];
        [menu popUpMenuPositioningItem:nil atLocation:NSMakePoint(100,NSMaxY(NSScreen.mainScreen.frame)-100) inView:nil];
    }
}

@interface MCMenuSwitch : NSSwitch
@property NSDictionary *payload;
// Do not override NSView.menu: its inherited setter retains the menu.
@property (weak) NSMenu *owningMenu;
@property BOOL originalState;
@end
@implementation MCMenuSwitch
- (void)toggle:(id)sender {
    self.state=self.originalState ? NSControlStateValueOn : NSControlStateValueOff;
    [self.owningMenu cancelTracking];
    sendPayload(self.payload);
}
@end

static NSImage *menuIcon(NSString *name, NSString *colorName) {
    NSString *path=[NSBundle.mainBundle.resourcePath stringByAppendingPathComponent:[NSString stringWithFormat:@"lucide/%@.svg",name]];
    NSString *svg=[NSString stringWithContentsOfFile:path encoding:NSUTF8StringEncoding error:nil];
    if (!svg) return nil;
    NSDictionary *colors=@{@"blue":NSColor.systemBlueColor,@"purple":NSColor.systemPurpleColor,@"green":NSColor.systemGreenColor,@"orange":NSColor.systemOrangeColor,@"pink":NSColor.systemPinkColor};
    NSColor *color=[(colors[colorName] ?: NSColor.controlAccentColor) colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    NSString *hex=[NSString stringWithFormat:@"#%02x%02x%02x",(int)round(color.redComponent*255),(int)round(color.greenComponent*255),(int)round(color.blueComponent*255)];
    svg=[svg stringByReplacingOccurrencesOfString:@"currentColor" withString:[colorName isEqual:@"monochrome"] ? @"#000000" : hex];
    NSImage *image=[[NSImage alloc] initWithData:[svg dataUsingEncoding:NSUTF8StringEncoding]];
    image.size=NSMakeSize(16,16);image.template=[colorName isEqual:@"monochrome"];
    return image;
}
static NSString *iconName(NSString *title) {
    for (NSArray *entry in @[@[@"Direct",@"globe"],@[@"Entire LAN",@"network"],@[@"VLESS",@"shield"],@[@"Internet VPN",@"route"],@[@"WireGuard",@"network"],@[@"Exceptions",@"list-filter"],@[@"Open dashboard",@"layout-dashboard"],@[@"Open in browser",@"external-link"],@[@"Refresh",@"refresh-cw"],@[@"Open settings",@"settings"],@[@"Activity",@"scroll-text"],@[@"Open latest",@"file-text"],@[@"Quit",@"power"],@[@"For ",@"clock"],@[@"Return",@"shield"]]) if ([title hasPrefix:entry[0]]) return entry[1];
    return nil;
}
static NSView *infoRow(NSString *title, BOOL heading, NSString *color) {
    CGFloat width=460;
    NSFont *font=[NSFont systemFontOfSize:heading ? 12 : 13 weight:heading ? NSFontWeightSemibold : NSFontWeightRegular];
    CGFloat height=ceil([title boundingRectWithSize:NSMakeSize(width-32,100) options:NSStringDrawingUsesLineFragmentOrigin attributes:@{NSFontAttributeName:font}].size.height)+8;
    NSView *view=[[NSView alloc] initWithFrame:NSMakeRect(0,0,width,MAX(24,height))];
    NSTextField *text=[NSTextField wrappingLabelWithString:title];text.font=font;text.textColor=heading ? NSColor.secondaryLabelColor : [NSColor.labelColor colorWithAlphaComponent:0.82];text.frame=NSMakeRect(16,4,width-32,view.frame.size.height-8);text.selectable=NO;
    [view addSubview:text];
    return view;
}
static void decorateMenu(NSMenu *menu, NSString *color, BOOL direct, BOOL root, NSString *thisMac, BOOL thisDirect) {
    BOOL info=root;
    for (NSMenuItem *item in menu.itemArray) {
        if (item.separatorItem) continue;
        if ([item.title isEqual:@"Direct access switch"] || [item.title isEqual:@"This Mac access switch"]) {
            BOOL local=[item.title isEqual:@"This Mac access switch"];
            BOOL active=local ? (thisDirect || direct) : direct;
            NSString *address=local ? thisMac : @"all";
            NSView *view=[[NSView alloc] initWithFrame:NSMakeRect(0,0,460,42)];
            NSTextField *title=[NSTextField labelWithString:local ? @"This Mac" : @"Direct internet"];
            title.font=[NSFont systemFontOfSize:13 weight:NSFontWeightSemibold];title.frame=NSMakeRect(16,22,300,17);[view addSubview:title];
            NSString *subtitle=local ? (direct ? @"Entire LAN already has direct access" : !thisMac.length ? @"Local address not found in router DHCP leases" : [NSString stringWithFormat:@"%@ · %@",thisMac,thisDirect ? @"direct access active" : @"only this Mac for 15 minutes"]) : (direct ? @"Entire LAN · direct access active" : @"Entire LAN · enable for 15 minutes");
            NSTextField *detail=[NSTextField labelWithString:subtitle];
            detail.font=[NSFont systemFontOfSize:11];detail.textColor=NSColor.secondaryLabelColor;detail.frame=NSMakeRect(16,5,360,16);[view addSubview:detail];
            MCMenuSwitch *toggle=[[MCMenuSwitch alloc] initWithFrame:NSMakeRect(402,8,42,28)];toggle.originalState=active;toggle.state=active ? NSControlStateValueOn : NSControlStateValueOff;toggle.enabled=item.enabled;toggle.owningMenu=menu;toggle.target=toggle;toggle.action=@selector(toggle:);toggle.accessibilityLabel=local ? @"Direct internet for this Mac" : @"Direct internet for entire LAN";
            toggle.payload=active ? @{@"kind":@"end_direct",@"address":address} : @{@"kind":@"direct",@"address":address,@"minutes":@15};[view addSubview:toggle];item.view=view;
            continue;
        }
        if (item.submenu) { info=NO;decorateMenu(item.submenu,color,direct,NO,thisMac,thisDirect); }
        if (info && !item.enabled && !item.submenu) {
            BOOL heading=[@[@"Router",@"Addresses",@"Connection"] containsObject:item.title];
            item.view=infoRow(item.title,heading,color);
        }
        NSString *name=iconName(item.title);if(name) { item.image=menuIcon(name,color); if (@available(macOS 27.0, *)) item.preferredImageVisibility=NSMenuItemImageVisibilityVisible; }
    }
}
void cockpit_style_menu(void *pointer, const char *theme, const char *color, bool direct, const char *thisMac, bool thisDirect) {
    NSString *appearance=[NSString stringWithUTF8String:theme];
    NSApp.appearance=[appearance isEqual:@"system"] ? nil : [NSAppearance appearanceNamed:[appearance isEqual:@"light"] ? NSAppearanceNameAqua : NSAppearanceNameDarkAqua];
    previewMenu=(__bridge NSMenu *)pointer;
    decorateMenu((__bridge NSMenu *)pointer,[NSString stringWithUTF8String:color],direct,YES,[NSString stringWithUTF8String:thisMac],thisDirect);
}
