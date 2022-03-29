use std::collections::HashMap;
use font::{self, Font, GlyphId};
use pdf::encoding::BaseEncoding;
use pdf::font::{Font as PdfFont, Widths, ToUnicodeMap};
use pdf::object::{Resolve, RcRef};
use pdf::error::PdfError;
use pdf_encoding::{Encoding, glyphname_to_unicode};
use std::sync::Arc;

#[derive(Debug)]
pub enum TextEncoding {
    CID,
    Cmap(HashMap<u16, (GlyphId, Option<char>)>)
}

pub struct FontEntry {
    pub font: Arc<dyn Font + Sync + Send>,
    pub pdf_font: RcRef<PdfFont>,
    pub encoding: TextEncoding,
    pub widths: Option<Widths>,
    pub is_cid: bool,
    pub name: String,
}
impl FontEntry {
    pub fn build(font: Arc<dyn Font + Sync + Send>, pdf_font: RcRef<PdfFont>, resolve: &impl Resolve) -> Result<FontEntry, PdfError> {
        let mut is_cid = pdf_font.is_cid();
        let encoding = pdf_font.encoding().clone();
        let base_encoding = encoding.as_ref().map(|e| &e.base);

        let mut to_unicode = t!(pdf_font.to_unicode(resolve).transpose());
        let encoding = if let Some(map) = pdf_font.cid_to_gid_map() {
            is_cid = true;
            let cmap = map.iter().enumerate().map(|(cid, &gid)| {
                let unicode = to_unicode.as_ref().and_then(|u| u.get(cid as u16)).and_then(|s| s.chars().next());
                (cid as u16, (GlyphId(gid as u32), unicode))
            }).collect();
            TextEncoding::Cmap(cmap)
        } else if base_encoding == Some(&BaseEncoding::IdentityH) {
            is_cid = true;
            TextEncoding::CID
        } else {
            let mut cmap = HashMap::new();
            let source_encoding = match base_encoding {
                Some(BaseEncoding::StandardEncoding) => Some(Encoding::AdobeStandard),
                Some(BaseEncoding::SymbolEncoding) => Some(Encoding::AdobeSymbol),
                Some(BaseEncoding::WinAnsiEncoding) => Some(Encoding::WinAnsiEncoding),
                Some(BaseEncoding::MacRomanEncoding) => Some(Encoding::MacRomanEncoding),
                Some(BaseEncoding::MacExpertEncoding) => Some(Encoding::AdobeExpert),
                ref e => {
                    warn!("unsupported pdf encoding {:?}", e);
                    None
                }
            };

            let font_encoding = font.encoding();
            debug!("{:?} -> {:?}", source_encoding, font_encoding);

            match (source_encoding, font_encoding) {
                (Some(source), Some(dest)) => {
                    if let Some(transcoder) = source.to(dest) {
                        let forward = source.forward_map().unwrap();
                        for b in 0 .. 256 {
                            if let Some(gid) = transcoder.translate(b).and_then(|cp| font.gid_for_codepoint(cp)) {
                                cmap.insert(b as u16, (gid, forward.get(b as u8)));
                                //debug!("{} -> {:?}", b, gid);
                            }
                        }
                    }
                },
                (Some(source), None) => {
                    if let Some(encoder) = source.to(Encoding::Unicode) {
                        for b in 0 .. 256 {
                            let unicode = encoder.translate(b as u32);
                            if let Some(gid) = unicode.and_then(|c| font.gid_for_unicode_codepoint(c)) {
                                cmap.insert(b, (gid, unicode.and_then(std::char::from_u32)));
                                //debug!("{} -> {:?}", b, gid);
                            }
                        }
                    }
                }
                _ => {
                    warn!("can't translate from text encoding {:?} to font encoding {:?}", base_encoding, font_encoding);
                    
                    // assuming same encoding
                    for cp in 0 .. 256 {
                        if let Some(gid) = font.gid_for_codepoint(cp) {
                            cmap.insert(cp as u16, (gid, std::char::from_u32(0xf000 + cp)));
                        }
                    }
                }
            }
            if let Some(encoding) = encoding {
                for (&cp, name) in encoding.differences.iter() {
                    //debug!("{} -> {}", cp, name);
                    match font.gid_for_name(&name) {
                        Some(gid) => {
                            let unicode = glyphname_to_unicode(name)
                                .or_else(|| name.find(".").and_then(|i| glyphname_to_unicode(&name[..i])))
                                .and_then(|s| s.chars().next());
                            cmap.insert(cp as u16, (gid, unicode));
                        }
                        None => info!("no glyph for name {}", name)
                    }
                }
            }
            //debug!("cmap: {:?}", cmap);
            //debug!("to_unicode: {:?}", to_unicode);
            if cmap.is_empty() {
                TextEncoding::CID
            } else {
                TextEncoding::Cmap(cmap)
            }
        };
        
        let widths = pdf_font.widths(resolve)?;
        let name = pdf_font.name.as_ref().ok_or_else(|| PdfError::Other { msg: "font has no name".into() })?.clone();
        Ok(FontEntry {
            font,
            pdf_font,
            encoding,
            is_cid,
            widths,
            name,
        })
    }
}
