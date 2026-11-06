//! Isolated native test window. No host, accounts, streaming, vault or network.
use eframe::egui;
use std::sync::mpsc;
struct Review { tray:crate::tray::Tray, tx:mpsc::Sender<String>, rx:mpsc::Receiver<String>, status:String }
pub fn run() -> eframe::Result {
    eframe::run_native("Secpar tray review",eframe::NativeOptions {
        viewport:egui::ViewportBuilder::default().with_title("Secpar tray review").with_inner_size([520.,220.]),..Default::default()
    },Box::new(|cc| {
        let(tx,rx)=mpsc::channel();let tray=crate::tray::Tray::new(cc).expect("test tray");
        let opener=tray.opener();opener.open();
        if let Some(output)=std::env::var_os("SECPAR_TRAY_REVIEW_RESULT") {
            let ctx=cc.egui_ctx.clone();let tx=tx.clone();
            std::thread::spawn(move|| {
                std::thread::sleep(std::time::Duration::from_millis(600));
                opener.hide();
                std::thread::sleep(std::time::Duration::from_millis(600));
                let hidden=!opener.visible();opener.open();
                std::thread::sleep(std::time::Duration::from_millis(600));
                let restored=opener.visible();opener.minimize();
                std::thread::sleep(std::time::Duration::from_millis(300));opener.open();
                std::thread::sleep(std::time::Duration::from_millis(300));
                let result=format!("hidden={hidden}; restored={restored}; minimized_restored={}",opener.visible());
                std::fs::write(output,&result).unwrap();let _=tx.send(result);ctx.request_repaint();
            });
        }
        Ok(Box::new(Review {tray,tx,rx,status:"Isolated tray test".into()}))
    }))
}
impl eframe::App for Review {
    fn update(&mut self,ctx:&egui::Context,_:&mut eframe::Frame) {
        while let Ok(s)=self.rx.try_recv(){self.status=s;}
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let _=self.tray.hide_on_close(ctx);
        egui::CentralPanel::default().show(ctx,|ui| {
            ui.heading("Secpar tray restore review");ui.label(&self.status);
            if ui.button("Hide, then run Open action").clicked() {
                let opener=self.tray.opener();let tx=self.tx.clone();let ctx=ctx.clone();
                opener.hide();
                std::thread::spawn(move||{
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let hidden=!opener.visible();opener.open();
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let result=format!("Hide succeeded: {hidden}; Open restored: {}",opener.visible());
                    let _=tx.send(result);ctx.request_repaint();
                });
            }
            if ui.button("Minimize, then run Open action").clicked() {
                let opener=self.tray.opener();let tx=self.tx.clone();let ctx=ctx.clone();opener.minimize();
                std::thread::spawn(move||{std::thread::sleep(std::time::Duration::from_millis(400));opener.open();
                    std::thread::sleep(std::time::Duration::from_millis(400));let _=tx.send(format!("Minimized Open restored: {}",opener.visible()));ctx.request_repaint();});
            }
        });
    }
}
