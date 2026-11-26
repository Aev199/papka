//! OLE clipboard file providers (RDP, virtual Shell items). Materialize into an
//! owned temporary directory; the application then uses its audited copy queue.
use std::{io,path::{Path,PathBuf}};

#[derive(Clone,Copy,Debug)]
pub struct Ticket {sequence:u32}
#[derive(Debug)]
pub struct StagedFiles {_directory:tempfile::TempDir,paths:Vec<PathBuf>}
impl StagedFiles {pub fn paths(&self)->&[PathBuf] {&self.paths}}

fn virtual_format(mut available:impl FnMut(&str)->bool)->bool {
    ["FileGroupDescriptorW","FileGroupDescriptor"].into_iter().any(&mut available)
}
fn unchanged(expected:u32,actual:u32)->io::Result<()> {
    if expected==0 || actual!=expected {
        Err(io::Error::new(io::ErrorKind::Interrupted,"Буфер обмена изменился; скопируйте файлы ещё раз"))
    }else{Ok(())}
}

/// Probe format availability only; never request remote file contents on the UI thread.
pub fn capture()->io::Result<Option<Ticket>> {
    #[cfg(windows)] {platform::capture()}
    #[cfg(not(windows))] {Ok(None)}
}
impl Ticket {
    pub fn materialize(self,owner:Option<isize>)->io::Result<StagedFiles> {
        #[cfg(windows)] {platform::materialize(self,owner)}
        #[cfg(not(windows))] {let _=owner;Err(io::Error::new(io::ErrorKind::Unsupported,"OLE clipboard requires Windows"))}
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface,IUnknown,PCWSTR};
    use windows::Win32::{Foundation::HWND,System::{Com::{CoCreateInstance,IDataObject,CLSCTX_ALL},
        Ole::{OleInitialize,OleUninitialize,OleGetClipboard}},UI::Shell::{IFileOperation,IShellItem,FileOperation,
        SHCreateItemFromParsingName,FOF_NOCONFIRMMKDIR,FOF_NOERRORUI,FOF_RENAMEONCOLLISION,FOF_SILENT,FOF_NOCONFIRMATION}};
    use windows_sys::Win32::System::DataExchange::{GetClipboardSequenceNumber,IsClipboardFormatAvailable,RegisterClipboardFormatW};

    struct Ole;
    impl Ole {fn enter()->io::Result<Self> {unsafe{OleInitialize(None)}.map_err(io::Error::other)?;Ok(Self)}}
    impl Drop for Ole {fn drop(&mut self){unsafe{OleUninitialize()};}}
    fn wide(value:&std::ffi::OsStr)->Vec<u16> {value.encode_wide().chain(Some(0)).collect()}
    fn sequence()->u32 {unsafe{GetClipboardSequenceNumber()}}
    pub(super) fn capture()->io::Result<Option<Ticket>> {
        let before=sequence();
        let present=virtual_format(|name|{
            let name:Vec<_>=name.encode_utf16().chain(Some(0)).collect();
            let format=unsafe{RegisterClipboardFormatW(name.as_ptr())};
            format!=0 && unsafe{IsClipboardFormatAvailable(format)}!=0
        });
        if !present {return Ok(None);}
        unchanged(before,sequence())?;
        Ok(Some(Ticket{sequence:before}))
    }

    #[windows::core::implement(windows::Win32::UI::Shell::IFileOperationProgressSink)]
    struct CopySink {failure:std::sync::Arc<std::sync::Mutex<Option<String>>>}
    impl CopySink {
        fn record(&self,hr:windows::core::HRESULT) {
            use windows::Win32::UI::Shell::{COPYENGINE_S_USER_IGNORED,COPYENGINE_S_NOT_HANDLED,COPYENGINE_S_PENDING,COPYENGINE_S_PENDING_DELETE};
            let skipped=[COPYENGINE_S_USER_IGNORED,COPYENGINE_S_NOT_HANDLED,COPYENGINE_S_PENDING,COPYENGINE_S_PENDING_DELETE].contains(&hr);
            if hr.is_err() || skipped {
                self.failure.lock().unwrap_or_else(|e|e.into_inner()).get_or_insert_with(||format!("Windows не получил один из файлов ({:#x})",hr.0));
            }
        }
    }
    #[allow(non_snake_case,unused_variables)]
    impl windows::Win32::UI::Shell::IFileOperationProgressSink_Impl for CopySink_Impl {
        fn StartOperations(&self)->windows::core::Result<()> {Ok(())}
        fn FinishOperations(&self,hr:windows::core::HRESULT)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn PreRenameItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,name:&PCWSTR)->windows::core::Result<()> {Ok(())}
        fn PostRenameItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,name:&PCWSTR,hr:windows::core::HRESULT,new:windows::core::Ref<IShellItem>)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn PreMoveItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,folder:windows::core::Ref<IShellItem>,name:&PCWSTR)->windows::core::Result<()> {Err(windows::core::Error::from_hresult(windows::core::HRESULT(0x80070005u32 as i32)))}
        fn PostMoveItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,folder:windows::core::Ref<IShellItem>,name:&PCWSTR,hr:windows::core::HRESULT,new:windows::core::Ref<IShellItem>)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn PreCopyItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,folder:windows::core::Ref<IShellItem>,name:&PCWSTR)->windows::core::Result<()> {Ok(())}
        fn PostCopyItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,folder:windows::core::Ref<IShellItem>,name:&PCWSTR,hr:windows::core::HRESULT,new:windows::core::Ref<IShellItem>)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn PreDeleteItem(&self,flags:u32,item:windows::core::Ref<IShellItem>)->windows::core::Result<()> {Err(windows::core::Error::from_hresult(windows::core::HRESULT(0x80070005u32 as i32)))}
        fn PostDeleteItem(&self,flags:u32,item:windows::core::Ref<IShellItem>,hr:windows::core::HRESULT,new:windows::core::Ref<IShellItem>)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn PreNewItem(&self,flags:u32,folder:windows::core::Ref<IShellItem>,name:&PCWSTR)->windows::core::Result<()> {Ok(())}
        fn PostNewItem(&self,flags:u32,folder:windows::core::Ref<IShellItem>,name:&PCWSTR,template:&PCWSTR,attrs:u32,hr:windows::core::HRESULT,new:windows::core::Ref<IShellItem>)->windows::core::Result<()> {self.record(hr);Ok(())}
        fn UpdateProgress(&self,total:u32,done:u32)->windows::core::Result<()> {Ok(())}
        fn ResetTimer(&self)->windows::core::Result<()> {Ok(())}
        fn PauseTimer(&self)->windows::core::Result<()> {Ok(())}
        fn ResumeTimer(&self)->windows::core::Result<()> {Ok(())}
    }

    fn copy_data(data:&IDataObject,target:&Path,owner:Option<isize>,silent:bool)->io::Result<()> {
        let target=wide(crate::path_utils::normalize_extended_path(target).as_os_str());
        unsafe {
            let operation:IFileOperation=CoCreateInstance(&FileOperation,None,CLSCTX_ALL).map_err(io::Error::other)?;
            let mut flags=FOF_NOCONFIRMMKDIR|FOF_NOERRORUI|FOF_RENAMEONCOLLISION;
            if silent {flags=flags|FOF_SILENT|FOF_NOCONFIRMATION;}
            operation.SetOperationFlags(flags).map_err(io::Error::other)?;
            if let Some(owner)=owner.filter(|h|*h!=0) {operation.SetOwnerWindow(HWND(owner as *mut _)).map_err(io::Error::other)?;}
            let folder:IShellItem=SHCreateItemFromParsingName(PCWSTR(target.as_ptr()),None).map_err(io::Error::other)?;
            let source:IUnknown=data.cast().map_err(io::Error::other)?;
            // CopyItems accepts IDataObject directly, including FileGroupDescriptor/FileContents.
            let failure=std::sync::Arc::new(std::sync::Mutex::new(None));
            let sink:windows::Win32::UI::Shell::IFileOperationProgressSink=CopySink{failure:failure.clone()}.into();
            let cookie=operation.Advise(&sink).map_err(io::Error::other)?;
            operation.CopyItems(&source,&folder).map_err(io::Error::other)?;
            let performed=operation.PerformOperations();
            let _=operation.Unadvise(cookie);
            if let Some(error)=failure.lock().unwrap_or_else(|e|e.into_inner()).take() {return Err(io::Error::other(error));}
            if operation.GetAnyOperationsAborted().map_err(io::Error::other)?.as_bool() {
                return Err(io::Error::new(io::ErrorKind::Interrupted,"Получение файлов из буфера отменено"));
            }
            performed.map_err(io::Error::other)?;
        }
        Ok(())
    }
    fn stage(data:&IDataObject,owner:Option<isize>,silent:bool)->io::Result<StagedFiles> {
        let directory=tempfile::Builder::new().prefix("Papka-clipboard-").tempdir()?;
        copy_data(data,directory.path(),owner,silent)?;
        let mut paths=std::fs::read_dir(directory.path())?.map(|e|e.map(|e|e.path())).collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        if paths.is_empty(){return Err(io::Error::other("Windows не вернул файлы из буфера обмена"));}
        Ok(StagedFiles{_directory:directory,paths})
    }
    pub(super) fn materialize(ticket:Ticket,owner:Option<isize>)->io::Result<StagedFiles> {
        // Own an STA; no raw COM interface crosses GPUI worker apartments.
        std::thread::Builder::new().name("papka-clipboard-sta".into()).spawn(move||{
            let _ole=Ole::enter()?;
            unchanged(ticket.sequence,sequence())?;
            let data=unsafe{OleGetClipboard()}.map_err(io::Error::other)?;
            unchanged(ticket.sequence,sequence())?;
            stage(&data,owner,false)
        })?.join().map_err(|_|io::Error::other("Windows clipboard worker failed"))?
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::System::Com::{IDataObject_Impl,FORMATETC,STGMEDIUM,STGMEDIUM_0,IEnumFORMATETC,IEnumSTATDATA,IAdviseSink};
        use windows::core::{HRESULT,Ref,BOOL};
        #[windows::core::implement(IDataObject)]
        struct VirtualData {descriptor:u16,contents:u16,group:Vec<u8>}
        fn not_supported()->windows::core::Error {windows::core::Error::from_hresult(HRESULT(0x80040064u32 as i32))}
        #[allow(non_snake_case)]
        impl IDataObject_Impl for VirtualData_Impl {
            fn GetData(&self,format:*const FORMATETC)->windows::core::Result<STGMEDIUM> {
                let format=unsafe{format.as_ref()}.ok_or_else(not_supported)?;
                if format.cfFormat==self.descriptor && format.tymed&1!=0 {
                    use windows_sys::Win32::{System::Memory::{GlobalAlloc,GlobalLock,GlobalUnlock,GMEM_MOVEABLE},Foundation::GlobalFree};
                    unsafe {
                        let handle=GlobalAlloc(GMEM_MOVEABLE,self.group.len());if handle.is_null(){return Err(not_supported());}
                        let pointer=GlobalLock(handle)as *mut u8;
                        if pointer.is_null(){GlobalFree(handle);return Err(not_supported());}
                        std::ptr::copy_nonoverlapping(self.group.as_ptr(),pointer,self.group.len());GlobalUnlock(handle);
                        return Ok(STGMEDIUM{tymed:1,u:STGMEDIUM_0{hGlobal:windows::Win32::Foundation::HGLOBAL(handle)},..Default::default()});
                    }
                }
                if format.cfFormat==self.contents && format.tymed&4!=0 {
                    let bytes:&[u8]=match format.lindex {0=>b"file",1=>b"",2=>b"nested",_=>return Err(not_supported())};
                    let stream=unsafe{windows::Win32::UI::Shell::SHCreateMemStream(Some(bytes))}.ok_or_else(not_supported)?;
                    return Ok(STGMEDIUM{tymed:4,u:STGMEDIUM_0{pstm:std::mem::ManuallyDrop::new(Some(stream))},..Default::default()});
                }
                Err(not_supported())
            }
            fn GetDataHere(&self,_format:*const FORMATETC,_medium:*mut STGMEDIUM)->windows::core::Result<()> {Err(not_supported())}
            fn QueryGetData(&self,format:*const FORMATETC)->HRESULT {
                match unsafe{format.as_ref()} {
                    Some(f)if(f.cfFormat==self.descriptor && f.tymed&1!=0)||(f.cfFormat==self.contents && f.tymed&4!=0)=>HRESULT(0),
                    _=>HRESULT(0x80040064u32 as i32),
                }
            }
            fn GetCanonicalFormatEtc(&self,_input:*const FORMATETC,_output:*mut FORMATETC)->HRESULT {HRESULT(0x80004001u32 as i32)}
            fn SetData(&self,_format:*const FORMATETC,medium:*const STGMEDIUM,release:BOOL)->windows::core::Result<()> {
                if release.as_bool()&&!medium.is_null(){unsafe{windows::Win32::System::Ole::ReleaseStgMedium(medium as *mut STGMEDIUM)};}
                Ok(())
            }
            fn EnumFormatEtc(&self,direction:u32)->windows::core::Result<IEnumFORMATETC> {
                if direction!=1{return Err(not_supported());}
                let formats=[FORMATETC{cfFormat:self.descriptor,dwAspect:1,lindex:-1,tymed:1,..Default::default()},
                    FORMATETC{cfFormat:self.contents,dwAspect:1,lindex:-1,tymed:4,..Default::default()}];
                unsafe{windows::Win32::UI::Shell::SHCreateStdEnumFmtEtc(&formats)}
            }
            fn DAdvise(&self,_format:*const FORMATETC,_flags:u32,_sink:Ref<IAdviseSink>)->windows::core::Result<u32> {Err(not_supported())}
            fn DUnadvise(&self,_connection:u32)->windows::core::Result<()> {Err(not_supported())}
            fn EnumDAdvise(&self)->windows::core::Result<IEnumSTATDATA> {Err(not_supported())}
        }
        #[test]
        fn virtual_file_streams_and_folder_work_without_any_hdrop_paths() {
            std::thread::spawn(||{
                let _ole=Ole::enter().unwrap();
                let register=|name:&str|{let name:Vec<_>=name.encode_utf16().chain(Some(0)).collect();(unsafe{RegisterClipboardFormatW(name.as_ptr())})as u16};
                let mut group=3u32.to_le_bytes().to_vec();
                for (name,directory,size)in [("Данные.txt",false,4),("Папка",true,0),("Папка\\nested.txt",false,6)] {
                    let mut descriptor=windows::Win32::UI::Shell::FILEDESCRIPTORW::default();
                    descriptor.dwFlags=0x80000004|if directory{0}else{0x40};descriptor.dwFileAttributes=if directory{16}else{128};
                    descriptor.nFileSizeLow=size;
                    let mut wide_name=[0u16;260];for (index,c)in name.encode_utf16().enumerate(){wide_name[index]=c;}
                    descriptor.cFileName=wide_name;
                    let bytes=unsafe{std::slice::from_raw_parts((&descriptor as *const windows::Win32::UI::Shell::FILEDESCRIPTORW).cast::<u8>(),std::mem::size_of_val(&descriptor))};
                    group.extend_from_slice(bytes);
                }
                let data:IDataObject=VirtualData{descriptor:register("FileGroupDescriptorW"),contents:register("FileContents"),group}.into();
                let hdrop=FORMATETC{cfFormat:15,dwAspect:1,lindex:-1,tymed:1,..Default::default()};
                assert!(unsafe{data.QueryGetData(&hdrop)}.is_err());
                let staged=stage(&data,None,true).unwrap();
                assert_eq!(std::fs::read(staged._directory.path().join("Данные.txt")).unwrap(),b"file");
                assert_eq!(std::fs::read(staged._directory.path().join("Папка/nested.txt")).unwrap(),b"nested");
            }).join().unwrap();
        }

        #[test]
        fn shell_data_object_materializes_unicode_file_and_folder_before_audited_copy() {
            std::thread::spawn(||{
                let _ole=Ole::enter().unwrap();
                let source=tempfile::tempdir().unwrap();
                let file=source.path().join("Файл.txt");std::fs::write(&file,b"file").unwrap();
                let folder=source.path().join("Папка");std::fs::create_dir(&folder).unwrap();
                std::fs::write(folder.join("nested.txt"),b"nested").unwrap();
                use windows::Win32::UI::Shell::{Common::ITEMIDLIST,SHParseDisplayName,SHCreateShellItemArrayFromIDLists,BHID_DataObject,ILFree};
                struct Pidls(Vec<*mut ITEMIDLIST>);
                impl Drop for Pidls{fn drop(&mut self){for p in &self.0{unsafe{ILFree(Some(*p))};}}}
                let mut pidls=Pidls(Vec::new());
                for path in [&file,&folder] {
                    let path=wide(path.as_os_str());let mut pidl=std::ptr::null_mut();
                    unsafe{SHParseDisplayName(PCWSTR(path.as_ptr()),None,&mut pidl,0,None)}.unwrap();pidls.0.push(pidl);
                }
                let raw:Vec<*const ITEMIDLIST>=pidls.0.iter().map(|p|*p as *const _).collect();
                let items=unsafe{SHCreateShellItemArrayFromIDLists(&raw)}.unwrap();
                let data:IDataObject=unsafe{items.BindToHandler(None,&BHID_DataObject)}.unwrap();
                let staged=stage(&data,None,true).unwrap();
                assert_eq!(staged.paths().len(),2);
                assert_eq!(std::fs::read(staged._directory.path().join("Файл.txt")).unwrap(),b"file");
                assert_eq!(std::fs::read(staged._directory.path().join("Папка/nested.txt")).unwrap(),b"nested");
                let destination=tempfile::tempdir().unwrap();
                let journal=crate::operation_journal::OperationJournal::open(destination.path().join("audit.sqlite")).unwrap();
                let mut zone=crate::operations::DropZone::default();for path in staged.paths(){zone.add(path).unwrap();}
                let results=zone.copy_to_audited(destination.path(),&Default::default(),&journal);
                assert!(results.iter().all(|(_,r)|r.is_ok()));
                assert_eq!(std::fs::read(destination.path().join("Папка/nested.txt")).unwrap(),b"nested");
                assert_eq!(std::fs::read(&file).unwrap(),b"file","provider source must never be moved");
                std::fs::remove_file(&file).unwrap();
                assert!(stage(&data,None,true).is_err(),"a partial provider failure must not become a successful import");
            }).join().unwrap();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn only_file_descriptor_formats_select_virtual_import() {
        assert!(virtual_format(|s|s=="FileGroupDescriptorW"));
        assert!(virtual_format(|s|s=="FileGroupDescriptor"));
        assert!(!virtual_format(|s|s=="CF_HDROP"||s=="UnicodeText"));
    }
    #[test] fn changed_or_unavailable_clipboard_is_not_pasted() {
        assert!(unchanged(5,5).is_ok());assert!(unchanged(5,6).is_err());assert!(unchanged(0,0).is_err());
    }
}
