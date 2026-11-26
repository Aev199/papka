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

#[derive(Debug)]
struct Descriptor {path:PathBuf,directory:bool,size:Option<u64>,modified:Option<u64>,attributes:u32}
fn bad_descriptor()->io::Error {io::Error::new(io::ErrorKind::InvalidData,"Некорректный список виртуальных файлов")}
fn parse_descriptors(bytes:&[u8])->io::Result<Vec<Descriptor>> {
    let count=u32::from_le_bytes(bytes.get(..4).ok_or_else(bad_descriptor)?.try_into().unwrap())as usize;
    if count==0 || count>100_000 || bytes.len()<4+count*592 {return Err(bad_descriptor());}
    let mut result=Vec::with_capacity(count);let mut names=std::collections::HashSet::new();
    for data in bytes[4..].chunks_exact(592).take(count) {
        let value=|offset|u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        let flags=value(0);let attributes=if flags&4!=0{value(36)}else{0};let directory=attributes&16!=0;
        let units:Vec<_>=data[72..].chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).take_while(|c|*c!=0).collect();
        let name=String::from_utf16(&units).map_err(|_|bad_descriptor())?.replace('\\',"/");
        let name=if directory{name.trim_end_matches('/')}else{name.as_str()};
        if name.is_empty() {return Err(bad_descriptor());}
        let mut path=PathBuf::new();
        for part in name.split('/') {
            let device=part.split('.').next().unwrap_or("").to_ascii_uppercase();
            let reserved=["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&device.as_str())
                || device.strip_prefix("COM").or_else(||device.strip_prefix("LPT")).is_some_and(|n|matches!(n,"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"¹"|"²"|"³"));
            if part.is_empty() || part=="." || part==".." || (part.ends_with('.')||part.ends_with(' ')) || reserved
                || part.chars().any(|c|c<' '||"<>:\"|?*".contains(c)) {return Err(bad_descriptor());}
            path.push(part);
        }
        if !names.insert(name.to_lowercase()){return Err(bad_descriptor());}
        let size=(!directory && flags&64!=0).then(||((value(64)as u64)<<32)|value(68)as u64);
        let modified=(flags&32!=0).then(||((value(60)as u64)<<32)|value(56)as u64);
        result.push(Descriptor{path,directory,size,modified,attributes});
    }
    Ok(result)
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
    struct Medium(windows::Win32::System::Com::STGMEDIUM);
    impl Drop for Medium {fn drop(&mut self){unsafe{windows::Win32::System::Ole::ReleaseStgMedium(&mut self.0)};}}
    fn format(name:&str,tymed:u32,index:i32)->windows::Win32::System::Com::FORMATETC {
        let name:Vec<_>=name.encode_utf16().chain(Some(0)).collect();
        windows::Win32::System::Com::FORMATETC{cfFormat:(unsafe{RegisterClipboardFormatW(name.as_ptr())})as u16,
            dwAspect:1,lindex:index,tymed,..Default::default()}
    }
    fn memory<T>(medium:&Medium,read:impl FnOnce(&[u8])->io::Result<T>)->io::Result<T> {
        if medium.0.tymed!=1{return Err(bad_descriptor());}
        use windows_sys::Win32::System::Memory::{GlobalLock,GlobalUnlock,GlobalSize};
        let handle=unsafe{medium.0.u.hGlobal}.0;
        struct Unlock(*mut core::ffi::c_void);
        impl Drop for Unlock {fn drop(&mut self){unsafe{GlobalUnlock(self.0)};}}
        let length=unsafe{GlobalSize(handle)};let pointer=unsafe{GlobalLock(handle)}as *const u8;
        if pointer.is_null(){return Err(io::Error::last_os_error());}
        let _unlock=Unlock(handle);
        read(unsafe{std::slice::from_raw_parts(pointer,length)})
    }
    fn descriptors(data:&IDataObject)->io::Result<Option<Vec<Descriptor>>> {
        for (name,unicode)in [("FileGroupDescriptorW",true),("FileGroupDescriptor",false)] {
            let request=format(name,1,-1);
            if unsafe{data.QueryGetData(&request)}.is_err(){continue;}
            let medium=Medium(unsafe{data.GetData(&request)}.map_err(io::Error::other)?);
            return memory(&medium,|bytes|{
                if bytes.len()>64*1024*1024{return Err(bad_descriptor());}
                if unicode {return parse_descriptors(bytes);}
                let count=u32::from_le_bytes(bytes.get(..4).ok_or_else(bad_descriptor)?.try_into().unwrap())as usize;
                if count==0||count>100_000||bytes.len()<4+332*count{return Err(bad_descriptor());}
                let mut wide=(count as u32).to_le_bytes().to_vec();
                for item in bytes[4..].chunks_exact(332).take(count) {
                    wide.extend_from_slice(&item[..72]);
                    let name=&item[72..];let length=name.iter().position(|b|*b==0).unwrap_or(name.len());
                    let mut units=[0u16;260];
                    let written=unsafe{windows_sys::Win32::Globalization::MultiByteToWideChar(0,0,name.as_ptr(),length as i32,units.as_mut_ptr(),259)};
                    if written==0{return Err(bad_descriptor());}
                    for unit in units {wide.extend_from_slice(&unit.to_le_bytes());}
                }
                parse_descriptors(&wide)
            }).map(Some);
        }
        Ok(None)
    }
    struct Progress {dialog:Option<windows::Win32::UI::Shell::IProgressDialog>,last:std::time::Instant}
    impl Progress {
        fn new(owner:Option<isize>,silent:bool)->io::Result<Self> {
            let dialog=if silent{None}else{unsafe{
                use windows::Win32::UI::Shell::{IProgressDialog,CLSID_ProgressDialog,PROGDLG_AUTOTIME};
                let dialog:IProgressDialog=CoCreateInstance(&CLSID_ProgressDialog,None,CLSCTX_ALL).map_err(io::Error::other)?;
                let title:Vec<_>="Получение файлов из буфера Windows".encode_utf16().chain(Some(0)).collect();
                dialog.SetTitle(PCWSTR(title.as_ptr())).map_err(io::Error::other)?;
                dialog.StartProgressDialog(owner.map(|h|HWND(h as *mut _)),None::<&IUnknown>,PROGDLG_AUTOTIME,None).map_err(io::Error::other)?;
                Some(dialog)
            }};
            Ok(Self{dialog,last:std::time::Instant::now()})
        }
        fn update(&mut self,path:&Path,done:u64,total:u64,force:bool)->io::Result<()> {
            if !force && self.last.elapsed()<std::time::Duration::from_millis(100){return Ok(());}
            self.last=std::time::Instant::now();
            if let Some(dialog)=&self.dialog {unsafe{
                if dialog.HasUserCancelled().as_bool(){return Err(io::Error::new(io::ErrorKind::Interrupted,"Получение файлов отменено"));}
                let name=wide(path.as_os_str());dialog.SetLine(1,PCWSTR(name.as_ptr()),true,None).map_err(io::Error::other)?;
                dialog.SetProgress64(done,total.max(1)).map_err(io::Error::other)?;
            }}
            Ok(())
        }
    }
    impl Drop for Progress {fn drop(&mut self){if let Some(dialog)=&self.dialog {let _=unsafe{dialog.StopProgressDialog()};}}}
    fn extract(data:&IDataObject,entries:&[Descriptor],target:&Path,owner:Option<isize>,silent:bool)->io::Result<()> {
        use std::io::Write;
        let bytes_total=entries.iter().filter(|e|!e.directory).try_fold(0u64,|sum,e|sum.checked_add(e.size?));
        let total=bytes_total.unwrap_or(entries.len()as u64);let mut completed=0u64;
        let mut progress=Progress::new(owner,silent)?;
        for (index,entry)in entries.iter().enumerate() {
            progress.update(&entry.path,if bytes_total.is_some(){completed}else{index as u64},total,true)?;
            let path=target.join(&entry.path);
            if entry.directory {std::fs::create_dir_all(&path)?;continue;}
            if let Some(parent)=path.parent(){std::fs::create_dir_all(parent)?;}
            let medium=Medium(unsafe{data.GetData(&format("FileContents",4,index as i32))}
                .or_else(|_|unsafe{data.GetData(&format("FileContents",1,index as i32))}).map_err(io::Error::other)?);
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
            let mut copied=0u64;
            match medium.0.tymed {
                4=>{
                    let stream=unsafe{&*medium.0.u.pstm}.as_ref().ok_or_else(bad_descriptor)?;
                    let mut buffer=[0u8;65536];
                    loop {
                        let mut read=0u32;
                        unsafe{stream.Read(buffer.as_mut_ptr().cast(),buffer.len()as u32,Some(&mut read))}.ok().map_err(io::Error::other)?;
                        if read as usize>buffer.len(){return Err(bad_descriptor());}if read==0{break;}
                        copied=copied.checked_add(read as u64).ok_or_else(bad_descriptor)?;
                        if entry.size.is_some_and(|size|copied>size){return Err(bad_descriptor());}
                        file.write_all(&buffer[..read as usize])?;
                        progress.update(&entry.path,if bytes_total.is_some(){completed.saturating_add(copied)}else{index as u64},total,false)?;
                    }
                }
                1=>memory(&medium,|bytes|{
                    let count=entry.size.map_or(Ok(bytes.len()),usize::try_from).map_err(|_|bad_descriptor())?;
                    if count>bytes.len(){return Err(bad_descriptor());}
                    for block in bytes[..count].chunks(65536) {
                        file.write_all(block)?;copied+=block.len()as u64;
                        progress.update(&entry.path,if bytes_total.is_some(){completed.saturating_add(copied)}else{index as u64},total,false)?;
                    }Ok(())
                })?,
                _=>return Err(io::Error::other("Неподдерживаемый поток виртуального файла")),
            }
            if entry.size.is_some_and(|size|size!=copied){return Err(io::Error::new(io::ErrorKind::UnexpectedEof,"Файл получен не полностью"));}
            file.sync_all()?;drop(file);completed=completed.saturating_add(copied);
        }
        // Apply directory dates only after writing their descendants.
        let mut metadata:Vec<_>=entries.iter().collect();metadata.sort_by_key(|e|std::cmp::Reverse(e.path.components().count()));
        for entry in metadata {
            let path=target.join(&entry.path);
            if let Some(ticks)=entry.modified {
                let time=filetime::FileTime::from_unix_time((ticks/10_000_000)as i64-11_644_473_600,((ticks%10_000_000)*100)as u32);
                filetime::set_file_mtime(&path,time)?;
            }
            if entry.attributes&6!=0 {
                use std::os::windows::fs::MetadataExt;
                let flags=std::fs::metadata(&path)?.file_attributes()|entry.attributes&6;
                let name=wide(path.as_os_str());
                if unsafe{windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(name.as_ptr(),flags)}==0{return Err(io::Error::last_os_error());}
            }
        }
        progress.update(Path::new("Готово"),total,total,true)?;
        Ok(())
    }

    fn stage(data:&IDataObject,owner:Option<isize>,silent:bool)->io::Result<StagedFiles> {
        let directory=tempfile::Builder::new().prefix("Papka-clipboard-").tempdir()?;
        if let Some(entries)=descriptors(data)? {extract(data,&entries,directory.path(),owner,silent)?;}
        else {copy_data(data,directory.path(),owner,silent)?;}
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

#[cfg(test)]
mod descriptor_tests {
    use super::*;
    fn group(names:&[&str])->Vec<u8> {
        let mut bytes=vec![0;4+names.len()*592];bytes[..4].copy_from_slice(&(names.len()as u32).to_le_bytes());
        for (i,name)in names.iter().enumerate(){
            let start=4+i*592;bytes[start..start+4].copy_from_slice(&68u32.to_le_bytes());
            for (j,c)in name.encode_utf16().enumerate(){bytes[start+72+j*2..start+74+j*2].copy_from_slice(&c.to_le_bytes());}
        }
        bytes
    }
    #[test] fn names_cannot_escape_staging_or_address_devices_and_streams() {
        for name in ["../file","/root","C:/file","file:stream","a/../file","NUL.txt","COM1","COM¹.txt","CONIN$","a.","a ","a//file",""] {
            assert!(parse_descriptors(&group(&[name])).is_err(),"{name}");
        }
        assert!(parse_descriptors(&group(&["A.txt","a.txt"])).is_err());
        assert!(parse_descriptors(&[255;4]).is_err());
    }
    #[test] fn unicode_relative_names_and_declared_sizes_are_decoded() {
        let bytes=group(&["Папка/Данные.txt"]);let entries=parse_descriptors(&bytes).unwrap();
        assert_eq!(entries[0].path,Path::new("Папка").join("Данные.txt"));
        assert_eq!(entries[0].size,Some(0));assert!(!entries[0].directory);
    }
}
