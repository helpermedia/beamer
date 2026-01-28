//! VST3 Plugin Factory implementation.
//!
//! Generic factory with inline component creation.

use std::ffi::c_void;
use std::marker::PhantomData;

use beamer_core::Config;
use vst3::com_scrape_types::MakeHeader;
use vst3::{Class, ComWrapper, Steinberg::*};

use crate::util::{copy_cstring, copy_wstring};
use crate::wrapper::Vst3Config;

/// VST3 Plugin Factory.
///
/// Generic over the component type C. Creates combined component instances
/// (IComponent + IEditController in one object).
pub struct Factory<C> {
    config: &'static Config,
    vst3_config: &'static Vst3Config,
    _marker: PhantomData<C>,
}

impl<C> Factory<C> {
    /// Create a new factory with the given configuration.
    pub const fn new(config: &'static Config, vst3_config: &'static Vst3Config) -> Self {
        Self {
            config,
            vst3_config,
            _marker: PhantomData,
        }
    }
}

/// Trait implemented by component types that can be constructed from plugin configs.
pub trait ComponentFactory: Class {
    fn create(config: &'static Config, vst3_config: &'static Vst3Config) -> Self;
}

impl<C> Class for Factory<C>
where
    C: ComponentFactory + 'static,
    C::Interfaces: MakeHeader<C, ComWrapper<C>>,
{
    type Interfaces = (IPluginFactory3,);
}

impl<C> IPluginFactoryTrait for Factory<C>
where
    C: ComponentFactory + 'static,
    C::Interfaces: MakeHeader<C, ComWrapper<C>>,
{
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = &mut *info;
        copy_cstring(self.config.vendor, &mut info.vendor);
        copy_cstring(self.config.url, &mut info.url);
        copy_cstring(self.config.email, &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;

        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        if self.vst3_config.has_controller() {
            2
        } else {
            1
        }
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        match index {
            0 => {
                let info = &mut *info;
                info.cid = self.vst3_config.component_uid;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Audio Module Class", &mut info.category);
                copy_cstring(self.config.name, &mut info.name);
                kResultOk
            }
            1 if self.vst3_config.has_controller() => {
                let info = &mut *info;
                info.cid = self.vst3_config.controller_uid.unwrap();
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Component Controller Class", &mut info.category);
                copy_cstring(self.config.name, &mut info.name);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }

        let requested_cid = &*(cid as *const TUID);

        // Check if request matches component or controller UID
        if *requested_cid != self.vst3_config.component_uid {
            if let Some(controller_uid) = self.vst3_config.controller_uid {
                if *requested_cid != controller_uid {
                    return kInvalidArgument;
                }
            } else {
                return kInvalidArgument;
            }
        }

        // Create component and query requested interface
        let component = ComWrapper::new(C::create(self.config, self.vst3_config));
        let unknown = component.as_com_ref::<FUnknown>().unwrap();
        let ptr = unknown.as_ptr();
        ((*(*ptr).vtbl).queryInterface)(ptr, iid as *const TUID, obj)
    }
}

impl<C> IPluginFactory2Trait for Factory<C>
where
    C: ComponentFactory + 'static,
    C::Interfaces: MakeHeader<C, ComWrapper<C>>,
{
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        match index {
            0 => {
                let info = &mut *info;
                info.cid = self.vst3_config.component_uid;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Audio Module Class", &mut info.category);
                copy_cstring(self.config.name, &mut info.name);
                info.classFlags = 0;
                copy_cstring(self.vst3_config.categories, &mut info.subCategories);
                copy_cstring(self.config.vendor, &mut info.vendor);
                copy_cstring(self.config.version, &mut info.version);
                copy_cstring("VST 3.8.0", &mut info.sdkVersion);
                kResultOk
            }
            1 if self.vst3_config.has_controller() => {
                let info = &mut *info;
                info.cid = self.vst3_config.controller_uid.unwrap();
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Component Controller Class", &mut info.category);
                copy_cstring(self.config.name, &mut info.name);
                info.classFlags = 1; // kComponentControllerClass
                copy_cstring("", &mut info.subCategories);
                copy_cstring(self.config.vendor, &mut info.vendor);
                copy_cstring(self.config.version, &mut info.version);
                copy_cstring("VST 3.8.0", &mut info.sdkVersion);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }
}

impl<C> IPluginFactory3Trait for Factory<C>
where
    C: ComponentFactory + 'static,
    C::Interfaces: MakeHeader<C, ComWrapper<C>>,
{
    unsafe fn getClassInfoUnicode(&self, index: i32, info: *mut PClassInfoW) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        match index {
            0 => {
                let info = &mut *info;
                info.cid = self.vst3_config.component_uid;
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Audio Module Class", &mut info.category);
                copy_wstring(self.config.name, &mut info.name);
                info.classFlags = 0;
                copy_cstring(self.vst3_config.categories, &mut info.subCategories);
                copy_wstring(self.config.vendor, &mut info.vendor);
                copy_wstring(self.config.version, &mut info.version);
                copy_wstring("VST 3.8.0", &mut info.sdkVersion);
                kResultOk
            }
            1 if self.vst3_config.has_controller() => {
                let info = &mut *info;
                info.cid = self.vst3_config.controller_uid.unwrap();
                info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
                copy_cstring("Component Controller Class", &mut info.category);
                copy_wstring(self.config.name, &mut info.name);
                info.classFlags = 1; // kComponentControllerClass
                copy_cstring("", &mut info.subCategories);
                copy_wstring(self.config.vendor, &mut info.vendor);
                copy_wstring(self.config.version, &mut info.version);
                copy_wstring("VST 3.8.0", &mut info.sdkVersion);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn setHostContext(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
}
