use super::*;
use egui::{Align2,FontId,Pos2,Rect,Response,Sense,Stroke,Vec2};

use crate::icons::{paint as icon,Icon};
const TEXT:Color32=Color32::from_rgb(233,235,237);
const MUTED:Color32=Color32::from_rgb(148,153,158);
const RED:Color32=Color32::from_rgb(242,105,94);

fn row(ui:&mut egui::Ui,kind:Icon,label:&str,value:&str,arrow:bool,checked:bool,danger:bool)->Response {
    let (rect,response)=ui.allocate_exact_size(Vec2::new(ui.available_width(),34.),Sense::click());
    if response.hovered() || response.has_focus(){ui.painter().rect_filled(rect,3.,Color32::from_white_alpha(12));}
    let color=if danger {RED}else{TEXT};
    icon(ui,kind,Rect::from_min_size(rect.min+egui::vec2(9.,7.),egui::vec2(20.,20.)),color);
    ui.painter().text(Pos2::new(rect.left()+39.,rect.center().y),Align2::LEFT_CENTER,label,FontId::proportional(14.),color);
    let right=rect.right()-12.;
    if checked || arrow {
        icon(ui,if checked {Icon::Check}else{Icon::Chevron},Rect::from_center_size(
            egui::pos2(right-6.,rect.center().y),egui::vec2(18.,18.)),if checked {design::ACCENT}else{MUTED});
    }
    if !value.is_empty(){ui.painter().text(egui::pos2(right-if arrow||checked {18.}else{0.},rect.center().y),Align2::RIGHT_CENTER,value,FontId::proportional(12.),MUTED);}
    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) const RESIZE_NOTICE_SHOW:std::time::Duration=std::time::Duration::from_millis(2500);
pub(crate) const RESIZE_NOTICE_HIDE:std::time::Duration=std::time::Duration::from_millis(8500);
pub(crate) const STRIP_HEIGHT:f32=28.;
pub(crate) const FOOTER_LEFT:f32=9.;
pub(crate) const AUDIO_OFF:&str="Host audio is off in settings";
const WAITING:&str="Waiting for stream stats";

/// Which part of the resize notice window an age falls in: hidden before 2.5 s, shown until 8.5 s, then cleared.
pub(crate) fn resize_notice_state(age:std::time::Duration)->(bool,bool) {
    (age>RESIZE_NOTICE_SHOW,age>RESIZE_NOTICE_HIDE)
}

/// The strip text. Before the first stats sample it only says it is waiting.
pub(crate) fn strip_text(size:[usize;2],fps:u32,status:&str,stats:Option<(Option<u16>,f32,f32)>,decoder:Option<&str>)->String {
    let Some((rtt,mbps,received))=stats else {return WAITING.into()};
    let mut parts=vec![format!("{} × {}",size[0],size[1]),format!("{fps} fps"),status.to_string()];
    if let Some(n)=rtt {parts.push(format!("RTT {n} ms"));}
    parts.push(format!("{mbps:.1} Mbps"));
    parts.push(format!("{received:.1} received fps"));
    if let Some(decoder)=decoder {parts.push(decoder.to_string());}
    parts.join("   ·   ")
}

/// 1px white line at alpha 23 across the inner width, 5px clear above and below.
fn divider(ui:&mut egui::Ui) {
    ui.add_space(5.);
    let (rect,_)=ui.allocate_exact_size(Vec2::new(ui.available_width(),1.),Sense::hover());
    ui.painter().rect_filled(rect,0.,Color32::from_white_alpha(23));
    ui.add_space(5.);
}

use crate::client_settings::{RESOLUTIONS,resolution_label};

impl App {
    pub(super) fn stream_menu(&mut self,ctx:&egui::Context) {
        let button=(!self.menu_button_hidden).then(||egui::Area::new(egui::Id::new("stream-menu-button")).order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(12.,12.)).show(ctx,|ui|{
                ui.spacing_mut().button_padding=egui::vec2(8.,8.);
                if ui.add_sized([40.,40.],egui::Button::image(egui::Image::new((self.mark.id(),egui::vec2(24.,24.))))
                    .fill(Color32::from_rgba_unmultiplied(25,27,29,135)).stroke(Stroke::NONE).corner_radius(6))
                    .on_hover_text("Stream menu").clicked(){self.stream_menu_open=!self.stream_menu_open;self.menu_page=0;}
            }));
        // hidden (escape keys + S): no button, and nothing on screen counts as hovering it
        self.stream_menu_button_bounds=Some(button.map_or(Rect::NOTHING,|b|b.response.rect));
        // A strip across the top, like Parsec's: stream stats, or the resize notice for an old host.
        let waiting=self.resize_wait.filter(|(at,_)|resize_notice_state(at.elapsed()).0);
        if waiting.is_some() && self.resize_wait.is_some_and(|(at,_)|resize_notice_state(at.elapsed()).1) {self.resize_wait=None;}
        let strip=self.texture.is_some() && (self.show_stream_stats || waiting.is_some());
        if strip {
            let left=if self.menu_button_hidden {12.} else {60.};
            egui::Area::new(egui::Id::new("stream-stats")).order(egui::Order::Middle).interactable(false)
                .fixed_pos(egui::pos2(0.,0.)).show(ctx,|ui|{
                    let width=ctx.content_rect().width();
                    egui::Frame::new().fill(Color32::from_rgba_unmultiplied(15,17,19,215)).inner_margin(egui::Margin{left:left as i8,right:left as i8,top:0,bottom:0}).show(ui,|ui|{
                        let inner=width-2.*left;
                        let text=if waiting.is_some() {
                            "This computer's Secpar can't resize a live stream yet. Update it there, then pick the resolution again.".to_string()
                        } else {
                            strip_text(self.dimensions,self.stream_fps,&self.status,self.stream_stats,self.client.active_decoder.as_deref())
                        };
                        ui.allocate_ui_with_layout(Vec2::new(inner,STRIP_HEIGHT),egui::Layout::centered_and_justified(egui::Direction::LeftToRight),|ui|{
                            ui.add(egui::Label::new(egui::RichText::new(text).size(13.).color(TEXT)).truncate());
                        });
                    });
                });
        }
        if !self.stream_menu_open {
            self.stream_menu_bounds=None;
            return;
        }
        let panel=egui::Area::new(egui::Id::new("stream-menu-panel")).order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(60.,if strip {34.}else{12.})).show(ctx,|ui|{
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(25,27,29,246)).corner_radius(5).inner_margin(6).show(ui,|ui|{
                    ui.set_width(250.);ui.spacing_mut().item_spacing.y=0.;
                    if self.menu_page==0 {
                        if row(ui,if self.fs.on {Icon::Windowed}else{Icon::Window},if self.fs.on {"Window"}else{"Fullscreen"},&self.client.escape.chord(keys::Escape::Fullscreen),false,false,false).clicked(){self.set_fullscreen(ctx,!self.fs.on);}
                        if row(ui,Icon::Decoder,"Decoder","",true,false,false).clicked(){self.menu_page=1;}
                        if row(ui,Icon::Display,"Resolution",&resolution_label(self.options.width,self.options.height),true,false,false).clicked(){self.menu_page=4;}
                        if self.displays.len()>1 && row(ui,Icon::Display,"Switch display","",true,false,false).clicked(){self.menu_page=2;}
                        if row(ui,Icon::Clipboard,"Share clipboard","",false,self.client.clipboard,false).clicked(){
                            self.client.clipboard=!self.client.clipboard;self.client.save();self.options.clipboard=self.client.clipboard;
                            self.clipboard=self.client.clipboard.then(secpar_clipboard::Bridge::start);self.send(Command::ClipboardSharing(self.client.clipboard));
                        }
                        if row(ui,Icon::Stats,"Stream stats","",false,self.show_stream_stats,false).clicked(){self.show_stream_stats=!self.show_stream_stats;self.stream_menu_open=false;}
                        if row(ui,Icon::Audio,"Audio","",true,false,false).clicked(){self.menu_page=3;}
                        if row(ui,Icon::Help,"Help","",false,false,false).clicked(){let _=open::that("https://secpar.win/help/#stream-menu");}
                        divider(ui);
                        if row(ui,Icon::Exit,"Disconnect",&self.client.escape.chord(keys::Escape::Disconnect),false,false,true).clicked(){self.capture(ctx,false);self.send(Command::Stop);self.stream_menu_open=false;}
                        ui.add_space(5.);ui.horizontal(|ui|{
                            ui.add_space(FOOTER_LEFT);ui.label(egui::RichText::new(concat!("Secpar ",env!("CARGO_PKG_VERSION"))).size(10.).color(MUTED));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.add_space(FOOTER_LEFT);ui.label(egui::RichText::new(format!("Escape keys {}",self.client.escape.label())).size(10.).color(MUTED));});
                        });
                    } else {
                        let page=self.menu_page;
                        let title=match page {1=>"Decoder",2=>"Displays",4=>"Resolution",_=>"Audio"};
                        if row(ui,Icon::Back,title,"",false,false,false).clicked(){self.menu_page=0;}
                        divider(ui);
                        if page==1 {
                            for preference in [video::Preference::Automatic,video::Preference::Software,video::Preference::HARDWARE] {
                                if preference==video::Preference::HARDWARE && video::hardware_available(ctx)!=Some(true){continue;}
                                if row(ui,Icon::Decoder,preference.label(),"",false,self.client.decoder==preference,false).clicked(){
                                    if self.client.decoder!=preference {self.client.decoder=preference;self.client.save();self.decoder=None;self.ensure_video_decoder(ctx);self.client.active_decoder=None;self.send(Command::Keyframe);}
                                    self.menu_page=0;
                                }
                            }
                            if let Some(active)=&self.client.active_decoder {ui.add_space(8.);ui.label(egui::RichText::new(active).size(11.).color(MUTED));}
                        } else if page==4 {
                            for (w,h) in RESOLUTIONS {
                                let current=self.options.width==w && self.options.height==h;
                                if row(ui,Icon::Display,&resolution_label(w,h),"",false,current,false).clicked(){
                                    if !current {self.options.width=w;self.options.height=h;self.send(Command::Size(w,h));self.resize_wait=Some((std::time::Instant::now(),(w,h)));}
                                    self.menu_page=0;
                                }
                            }
                        } else if page==2 {
                            let mut selected=None;
                            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-90.).max(80.)).show(ui,|ui|{
                                for (id,name) in &self.displays {if row(ui,Icon::Display,name,"",false,self.display==*id,false).clicked(){selected=Some(id.clone());}}
                            });
                            if let Some(id)=selected {self.display=id.clone();self.send(Command::Display(id));self.menu_page=0;}
                        } else {
                            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-130.).max(80.)).show(ui,|ui|{
                                ui.add_space(8.);
                                if let Some(note)=&self.notice {ui.label(egui::RichText::new(note).size(12.).color(RED));}
                                if let Some(error)=&self.audio_error {ui.label(error);}
                                if self.audio.is_none() && self.audio_error.is_none() {ui.label(egui::RichText::new(AUDIO_OFF).size(12.).color(MUTED));}
                                if let Some(audio)=&self.audio {let details=audio.details();ui.label(egui::RichText::new(&details).size(12.).color(TEXT));if row(ui,Icon::Clipboard,"Copy details","",false,false,false).clicked(){ui.ctx().copy_text(details);}}
                                if self.client.audio && row(ui,Icon::Audio,"Reopen output","",false,false,false).clicked(){self.open_audio();}
                            });
                        }
                    }
                });
            });
        self.stream_menu_bounds=Some(panel.response.rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn resize_notice_window() {
        assert_eq!(resize_notice_state(Duration::from_millis(2500)),(false,false));
        assert_eq!(resize_notice_state(Duration::from_millis(2501)),(true,false));
        assert_eq!(resize_notice_state(Duration::from_millis(8500)),(true,false));
        assert_eq!(resize_notice_state(Duration::from_millis(8501)),(true,true));
    }
    #[test]
    fn strip_waits_for_first_sample() {
        assert_eq!(strip_text([1280,720],60,"Connected",None,None),"Waiting for stream stats");
    }
    #[test]
    fn strip_joins_parts() {
        let t=strip_text([1280,720],60,"Connected",Some((Some(2),18.44,59.96)),Some("H.264 · software"));
        assert_eq!(t,"1280 × 720   ·   60 fps   ·   Connected   ·   RTT 2 ms   ·   18.4 Mbps   ·   60.0 received fps   ·   H.264 · software");
        let t=strip_text([1280,720],60,"Connected",Some((None,1.,2.)),None);
        assert!(!t.contains("RTT") && t.ends_with("2.0 received fps"));
    }
    #[test]
    fn menu_strings() {
        assert_eq!(AUDIO_OFF,"Host audio is off in settings");
        assert_eq!(STRIP_HEIGHT,28.);
        assert_eq!(6.+FOOTER_LEFT,15.);
    }
}
